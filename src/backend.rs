//! Graphics backends: turn a rendered RGBA framebuffer into terminal output.
//!
//! The game renders its scene into a `Framebuffer` with no knowledge of how it
//! reaches the screen — that is this module's job. A `Backend` is fully decoupled
//! behind a trait so backends are swappable at runtime (press Tab in-game):
//!
//! - [`KittyBackend`] transmits the framebuffer as a scaled, double-buffered
//!   Kitty graphics image (pixel-perfect, terminal must speak the protocol).
//! - [`TextBackend`] samples the framebuffer into Unicode half-block cells
//!   (works in any terminal).
//!
//! Contract: `present` and `teardown` each fully own `out` — they clear it and
//! write a complete, flushable byte sequence. The caller appends the status line
//! and flushes once, so presentation stays atomic.

use crate::framebuffer::Framebuffer;
use crate::kitty;

pub trait Backend {
    /// Human-readable name (shown in the help menu / status).
    fn name(&self) -> &'static str;

    /// Encode `fb` into `out` for display. `cols`/`play_rows` are the terminal
    /// cell area the image should fill (full width × all rows but the status
    /// row). `full` requests a complete repaint (after a backend switch or
    /// resize) rather than an incremental update.
    fn present(
        &mut self,
        out: &mut Vec<u8>,
        fb: &Framebuffer,
        cols: u16,
        play_rows: u16,
        full: bool,
    );

    /// Erase this backend's output before another backend takes over.
    fn teardown(&mut self, out: &mut Vec<u8>);
}

/// Scaled, double-buffered Kitty graphics image (the default backend).
pub struct KittyBackend {
    b64: Vec<u8>,
    draw_id: u32,
}

impl KittyBackend {
    pub fn new() -> Self {
        KittyBackend { b64: Vec::new(), draw_id: kitty::BUF_A }
    }
}

impl Default for KittyBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for KittyBackend {
    fn name(&self) -> &'static str {
        "kitty"
    }

    fn present(&mut self, out: &mut Vec<u8>, fb: &Framebuffer, cols: u16, play_rows: u16, _full: bool) {
        // Transmit+display the new buffer, then delete the previous one — so the
        // terminal always has one complete image (no swap flicker).
        kitty::present_rgba(
            out,
            self.draw_id,
            fb.width,
            fb.height,
            cols as usize,
            play_rows as usize,
            &fb.px,
            &mut self.b64,
        );
        let other = kitty::BUF_A + kitty::BUF_B - self.draw_id;
        kitty::append_delete(out, other);
        self.draw_id = other;
    }

    fn teardown(&mut self, out: &mut Vec<u8>) {
        out.clear();
        out.extend_from_slice(kitty::delete_all());
    }
}

// ---------------------------------------------------------------------------
// Text backend — half-block cells, works in any terminal
// ---------------------------------------------------------------------------

/// The upper-half block; its foreground paints the top pixel of the cell and its
/// background the bottom pixel, giving two vertical pixels per character cell.
const HALF_BLOCK: &str = "\u{2580}"; // ▀

/// Samples the framebuffer into terminal cells. Each cell is one `▀` whose fg is
/// the top sub-pixel and bg the bottom sub-pixel, so a `cols × rows` grid shows
/// `cols × 2·rows` colored pixels. Full-frame redraw with SGR run-minimization
/// (a color escape is emitted only when it changes from the previous cell), so
/// flat regions (sky, walls) cost ~3 bytes/cell — cheap enough for 60fps.
pub struct TextBackend;

impl TextBackend {
    pub fn new() -> Self {
        TextBackend
    }
}

impl Default for TextBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Nearest-neighbour sample of the framebuffer at grid cell (`gx`,`gy`) of a
/// `grid_w × grid_h` sampling grid. Returns RGB.
#[inline]
fn sample(fb: &Framebuffer, gx: usize, gy: usize, grid_w: usize, grid_h: usize) -> (u8, u8, u8) {
    let fx = (gx * fb.width / grid_w).min(fb.width - 1);
    let fy = (gy * fb.height / grid_h).min(fb.height - 1);
    let i = (fy * fb.width + fx) * 4;
    (fb.px[i], fb.px[i + 1], fb.px[i + 2])
}

/// Append `ESC[<base>;2;r;g;b m` (base 38 = fg, 48 = bg) for a truecolor SGR.
#[inline]
fn push_sgr(out: &mut Vec<u8>, base: u8, (r, g, b): (u8, u8, u8)) {
    use std::io::Write;
    let _ = write!(out, "\x1b[{base};2;{r};{g};{b}m");
}

impl Backend for TextBackend {
    fn name(&self) -> &'static str {
        "text"
    }

    fn present(&mut self, out: &mut Vec<u8>, fb: &Framebuffer, cols: u16, play_rows: u16, _full: bool) {
        out.clear();
        out.extend_from_slice(b"\x1b[H");
        let cw = cols as usize;
        let ch = play_rows as usize;
        if cw == 0 || ch == 0 || fb.width == 0 || fb.height == 0 {
            return;
        }
        // The sampling grid is cols wide × 2·rows tall (two pixels per cell).
        let grid_w = cw;
        let grid_h = ch * 2;
        // SGR state persists across cursor moves, so we only re-emit on change.
        let mut cur_fg: Option<(u8, u8, u8)> = None;
        let mut cur_bg: Option<(u8, u8, u8)> = None;
        for cy in 0..ch {
            use std::io::Write;
            let _ = write!(out, "\x1b[{};1H", cy + 1); // start of this cell row
            for cx in 0..cw {
                let top = sample(fb, cx, cy * 2, grid_w, grid_h);
                let bot = sample(fb, cx, cy * 2 + 1, grid_w, grid_h);
                if cur_fg != Some(top) {
                    push_sgr(out, 38, top);
                    cur_fg = Some(top);
                }
                if cur_bg != Some(bot) {
                    push_sgr(out, 48, bot);
                    cur_bg = Some(bot);
                }
                out.extend_from_slice(HALF_BLOCK.as_bytes());
            }
        }
        out.extend_from_slice(b"\x1b[0m");
    }

    fn teardown(&mut self, out: &mut Vec<u8>) {
        out.clear();
        out.extend_from_slice(b"\x1b[0m\x1b[2J");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framebuffer::Rgba;

    fn fb_2x2() -> Framebuffer {
        // top row red/green, bottom row blue/white
        let mut fb = Framebuffer::new(2, 2);
        fb.set(0, 0, Rgba::rgb(255, 0, 0));
        fb.set(1, 0, Rgba::rgb(0, 255, 0));
        fb.set(0, 1, Rgba::rgb(0, 0, 255));
        fb.set(1, 1, Rgba::rgb(255, 255, 255));
        fb
    }

    #[test]
    fn text_cell_maps_top_to_fg_and_bottom_to_bg() {
        // 2x2 fb -> 2 cols x 1 row, grid 2x2 maps 1:1.
        let mut out = Vec::new();
        TextBackend::new().present(&mut out, &fb_2x2(), 2, 1, true);
        let s = String::from_utf8(out).unwrap();
        // cell 0: fg=red(top), bg=blue(bottom); cell 1: fg=green, bg=white
        assert!(s.contains("\x1b[38;2;255;0;0m"), "cell0 fg red missing: {s:?}");
        assert!(s.contains("\x1b[48;2;0;0;255m"), "cell0 bg blue missing");
        assert!(s.contains("\x1b[38;2;0;255;0m"), "cell1 fg green missing");
        assert!(s.contains("\x1b[48;2;255;255;255m"), "cell1 bg white missing");
        assert_eq!(s.matches('\u{2580}').count(), 2, "should emit one half-block per cell");
        assert!(s.ends_with("\x1b[0m"), "should reset SGR at the end");
    }

    #[test]
    fn text_minimizes_sgr_on_flat_color() {
        // A uniform framebuffer: first cell sets fg+bg, the rest reuse them.
        let mut fb = Framebuffer::new(4, 2);
        fb.clear(Rgba::rgb(10, 20, 30));
        let mut out = Vec::new();
        TextBackend::new().present(&mut out, &fb, 4, 1, true);
        let s = String::from_utf8(out).unwrap();
        // Only one fg and one bg SGR for the whole flat row of 4 cells.
        assert_eq!(s.matches("38;2;10;20;30m").count(), 1, "fg should be set once");
        assert_eq!(s.matches("48;2;10;20;30m").count(), 1, "bg should be set once");
        assert_eq!(s.matches('\u{2580}').count(), 4);
    }

    #[test]
    fn text_handles_size_mismatch_without_panic() {
        // Large cell grid vs small framebuffer (upsampling) must not index OOB.
        let mut fb = Framebuffer::new(3, 5);
        fb.clear(Rgba::rgb(1, 2, 3));
        let mut out = Vec::new();
        TextBackend::new().present(&mut out, &fb, 40, 12, true);
        assert!(!out.is_empty());
    }
}

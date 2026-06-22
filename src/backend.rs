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

/// A character layer (the player sprite, or an effect clip) stamped over the
/// cell grid by the character backends. `col`/`row` are the top-left cell;
/// spaces are transparent. `tint` is the uniform color the colored backends use
/// (`None` = per-glyph palette, i.e. Munchii's beagle colors). `z` is the draw
/// depth: higher `z` wins where layers overlap, so an effect can be authored to
/// sit behind or in front of the player and other sprites.
pub struct Overlay<'a> {
    pub lines: &'a [String],
    pub col: i32,
    pub row: i32,
    pub tint: Option<(u8, u8, u8)>,
    pub z: i32,
}

impl Overlay<'_> {
    /// The glyph covering cell (cx, cy), or None if uncovered/transparent.
    pub fn at(&self, cx: usize, cy: usize) -> Option<char> {
        let r = cy as i32 - self.row;
        let c = cx as i32 - self.col;
        if r < 0 || c < 0 {
            return None;
        }
        let ch = self.lines.get(r as usize)?.chars().nth(c as usize)?;
        if ch == ' ' {
            None
        } else {
            Some(ch)
        }
    }
}

/// Highest-z glyph (+ its layer tint) covering cell (cx, cy), or None. Layers
/// always cover the framebuffer scene, so any overlay draws over walls; `z`
/// only orders the overlays against each other.
fn top_glyph(overlays: &[Overlay], cx: usize, cy: usize) -> Option<(char, Option<(u8, u8, u8)>)> {
    overlays
        .iter()
        .filter_map(|o| o.at(cx, cy).map(|g| (o.z, g, o.tint)))
        .max_by_key(|(z, _, _)| *z)
        .map(|(_, g, t)| (g, t))
}

pub trait Backend {
    /// Human-readable name (shown in the help menu / status).
    fn name(&self) -> &'static str;

    /// True if this backend draws the player as a character sprite overlay
    /// (so the caller should NOT also draw the player into the framebuffer).
    fn draws_overlay(&self) -> bool {
        false
    }

    /// Encode `fb` into `out` for display. `cols`/`play_rows` are the terminal
    /// cell area the image fills. `full` requests a complete repaint. `overlays`
    /// are the character layers (player + effects) the character backends stamp
    /// on top, ordered by each layer's `z`.
    fn present(
        &mut self,
        out: &mut Vec<u8>,
        fb: &Framebuffer,
        cols: u16,
        play_rows: u16,
        full: bool,
        overlays: &[Overlay],
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

    fn present(&mut self, out: &mut Vec<u8>, fb: &Framebuffer, cols: u16, play_rows: u16, _full: bool, _overlays: &[Overlay]) {
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

    fn present(&mut self, out: &mut Vec<u8>, fb: &Framebuffer, cols: u16, play_rows: u16, _full: bool, _overlays: &[Overlay]) {
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

// ---------------------------------------------------------------------------
// ASCII backends — characters only, retro look
// ---------------------------------------------------------------------------

/// Fine brightness ramp (dark → bright) for the colored ASCII backend.
const RAMP_FINE: &[u8] = b" .:-=+*#%@";
/// Coarse ramp for the bare monochrome backend — fewer, chunkier levels.
const RAMP_COARSE: &[u8] = b" .:+#";

/// Rec.601 luma of an RGB triple, 0..=255.
#[inline]
fn luma((r, g, b): (u8, u8, u8)) -> u32 {
    (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000
}

/// Render the framebuffer (+ optional sprite overlay) to plain text rows — no
/// escape codes, no color — for screenshots and docs (the mono tier's look).
pub fn mono_text(fb: &Framebuffer, cols: usize, play_rows: usize, overlays: &[Overlay]) -> String {
    let mut s = String::with_capacity((cols + 1) * play_rows);
    if cols == 0 || play_rows == 0 || fb.width == 0 || fb.height == 0 {
        return s;
    }
    for cy in 0..play_rows {
        for cx in 0..cols {
            let ch = match top_glyph(overlays, cx, cy) {
                Some((g, _)) => g,
                None => ramp_glyph(RAMP_COARSE, luma(sample(fb, cx, cy, cols, play_rows))) as char,
            };
            s.push(ch);
        }
        s.push('\n');
    }
    s
}

#[inline]
fn ramp_glyph(ramp: &[u8], lum: u32) -> u8 {
    ramp[(lum as usize * (ramp.len() - 1)) / 255]
}

/// Colored ASCII art: one glyph per cell from a brightness ramp, each drawn in
/// its source pixel color (truecolor fg, emitted only on change). Reads as a
/// recognizable, colorful character rendering of the scene.
pub struct AsciiBackend;

impl AsciiBackend {
    pub fn new() -> Self {
        AsciiBackend
    }
}

impl Default for AsciiBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for AsciiBackend {
    fn name(&self) -> &'static str {
        "ascii"
    }

    fn draws_overlay(&self) -> bool {
        true
    }

    fn present(&mut self, out: &mut Vec<u8>, fb: &Framebuffer, cols: u16, play_rows: u16, _full: bool, overlays: &[Overlay]) {
        out.clear();
        out.extend_from_slice(b"\x1b[H");
        let cw = cols as usize;
        let ch = play_rows as usize;
        if cw == 0 || ch == 0 || fb.width == 0 || fb.height == 0 {
            return;
        }
        let mut cur_fg: Option<(u8, u8, u8)> = None;
        let mut tmp = [0u8; 4];
        for cy in 0..ch {
            use std::io::Write;
            let _ = write!(out, "\x1b[{};1H", cy + 1);
            for cx in 0..cw {
                // A character layer (player or effect) overrides the scene where
                // it covers a cell: effects use their tint, Munchii his beagle
                // palette; otherwise the brightness-ramped scene.
                let (glyph, col): (char, (u8, u8, u8)) = match top_glyph(overlays, cx, cy) {
                    Some((g, Some(t))) => (g, t),
                    Some((g, None)) => (g, crate::munchii::beagle_rgb(g)),
                    None => {
                        let c = sample(fb, cx, cy, cw, ch);
                        (ramp_glyph(RAMP_FINE, luma(c)) as char, c)
                    }
                };
                if cur_fg != Some(col) {
                    push_sgr(out, 38, col);
                    cur_fg = Some(col);
                }
                out.extend_from_slice(glyph.encode_utf8(&mut tmp).as_bytes());
            }
        }
        out.extend_from_slice(b"\x1b[0m");
    }

    fn teardown(&mut self, out: &mut Vec<u8>) {
        out.clear();
        out.extend_from_slice(b"\x1b[0m\x1b[2J");
    }
}

/// The bare-minimum renderer: plain black-and-white ASCII, a coarse ramp, and
/// *no* color escapes at all (terminal default fg). The lightest, simplest
/// backend — a teletype rendering of the scene.
pub struct MonoBackend;

impl MonoBackend {
    pub fn new() -> Self {
        MonoBackend
    }
}

impl Default for MonoBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for MonoBackend {
    fn name(&self) -> &'static str {
        "mono"
    }

    fn draws_overlay(&self) -> bool {
        true
    }

    fn present(&mut self, out: &mut Vec<u8>, fb: &Framebuffer, cols: u16, play_rows: u16, _full: bool, overlays: &[Overlay]) {
        out.clear();
        out.extend_from_slice(b"\x1b[H\x1b[0m"); // default colors, no SGR per cell
        let cw = cols as usize;
        let ch = play_rows as usize;
        if cw == 0 || ch == 0 || fb.width == 0 || fb.height == 0 {
            return;
        }
        let mut tmp = [0u8; 4];
        for cy in 0..ch {
            use std::io::Write;
            let _ = write!(out, "\x1b[{};1H", cy + 1);
            for cx in 0..cw {
                // Character layers (B&W) where they cover a cell, else the scene.
                match top_glyph(overlays, cx, cy) {
                    Some((g, _)) => out.extend_from_slice(g.encode_utf8(&mut tmp).as_bytes()),
                    None => out.push(ramp_glyph(RAMP_COARSE, luma(sample(fb, cx, cy, cw, ch)))),
                }
            }
        }
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
        TextBackend::new().present(&mut out, &fb_2x2(), 2, 1, true, &[]);
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
        TextBackend::new().present(&mut out, &fb, 4, 1, true, &[]);
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
        TextBackend::new().present(&mut out, &fb, 40, 12, true, &[]);
        assert!(!out.is_empty());
    }

    #[test]
    fn ascii_maps_brightness_to_ramp_and_colors_glyphs() {
        // White -> brightest ramp glyph, drawn in the source color.
        let mut white = Framebuffer::new(4, 2);
        white.clear(Rgba::rgb(255, 255, 255));
        let mut out = Vec::new();
        AsciiBackend::new().present(&mut out, &white, 4, 1, true, &[]);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b[38;2;255;255;255m"), "glyph should be colored by pixel");
        assert_eq!(s.matches('@').count(), 4, "white -> 4 '@' (fine ramp max)");

        // Black -> spaces.
        let mut black = Framebuffer::new(4, 2);
        black.clear(Rgba::rgb(0, 0, 0));
        let mut out2 = Vec::new();
        AsciiBackend::new().present(&mut out2, &black, 4, 1, true, &[]);
        let g: String = strip_csi(&String::from_utf8(out2).unwrap());
        assert_eq!(g.matches(' ').count(), 4, "black -> 4 spaces");
    }

    #[test]
    fn ascii_glyph_stream_is_pure_ascii() {
        let mut fb = Framebuffer::new(8, 8);
        fb.clear(Rgba::rgb(90, 102, 140));
        fb.fill_rect(2, 2, 3, 3, Rgba::rgb(240, 200, 80));
        let mut out = Vec::new();
        AsciiBackend::new().present(&mut out, &fb, 8, 4, true, &[]);
        let s = String::from_utf8(out).unwrap();
        for ch in strip_csi(&s).chars() {
            assert!(RAMP_FINE.contains(&(ch as u8)), "non-ramp glyph {ch:?}");
        }
    }

    #[test]
    fn overlay_stamps_munchii_over_the_scene() {
        let lines = ["@b".to_string(), "c .".to_string()];
        let ov = [Overlay { lines: &lines, col: 1, row: 0, tint: None, z: 0 }];
        let mut fb = Framebuffer::new(8, 8);
        fb.clear(Rgba::rgb(0, 0, 0));
        // mono: the sprite glyphs appear (transparent spaces don't erase)
        let mut out = Vec::new();
        MonoBackend::new().present(&mut out, &fb, 8, 2, true, &ov);
        let s = strip_csi(&String::from_utf8(out).unwrap());
        assert!(s.contains('@') && s.contains('b') && s.contains('c'), "stamped: {s:?}");
        // ascii: the nose '@' is drawn in the beagle near-black color (tint None)
        let mut out2 = Vec::new();
        AsciiBackend::new().present(&mut out2, &fb, 8, 2, true, &ov);
        let raw = String::from_utf8(out2).unwrap();
        let (r, g, b) = crate::munchii::beagle_rgb('@');
        assert!(raw.contains(&format!("38;2;{r};{g};{b}m")), "nose should be beagle-colored");
    }

    #[test]
    fn effect_tint_and_z_win_over_player() {
        // A tinted effect layer at higher z overrides a Munchii layer beneath it.
        let player = ["X".to_string()];
        let fx = ["E".to_string()];
        let ovs = [
            Overlay { lines: &player, col: 0, row: 0, tint: None, z: 0 },
            Overlay { lines: &fx, col: 0, row: 0, tint: Some((9, 8, 7)), z: 5 },
        ];
        let mut fb = Framebuffer::new(4, 4);
        fb.clear(Rgba::rgb(0, 0, 0));
        let mut out = Vec::new();
        AsciiBackend::new().present(&mut out, &fb, 4, 2, true, &ovs);
        let raw = String::from_utf8(out).unwrap();
        assert!(raw.contains("38;2;9;8;7m"), "higher-z effect tint should win");
        assert!(raw.contains('E') && !raw.contains('X'), "effect glyph on top");
    }

    #[test]
    fn mono_emits_no_color_and_uses_coarse_ramp() {
        let mut fb = Framebuffer::new(4, 2);
        fb.clear(Rgba::rgb(255, 255, 255));
        let mut out = Vec::new();
        MonoBackend::new().present(&mut out, &fb, 4, 1, true, &[]);
        let s = String::from_utf8(out).unwrap();
        // No truecolor SGR anywhere — it's plain black & white.
        assert!(!s.contains("38;2;"), "mono must not set fg color: {s:?}");
        assert!(!s.contains("48;2;"), "mono must not set bg color");
        // white -> coarsest ramp max ('#')
        let last = *RAMP_COARSE.last().unwrap() as char;
        assert_eq!(s.matches(last).count(), 4, "white -> 4 '{last}'");
        // every glyph is from the coarse ramp
        for ch in strip_csi(&s).chars() {
            assert!(RAMP_COARSE.contains(&(ch as u8)), "non-ramp glyph {ch:?}");
        }
    }

    // crude CSI stripper for tests
    fn strip_csi(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            if c == '\x1b' {
                if it.peek() == Some(&'[') {
                    it.next();
                    while let Some(&d) = it.peek() {
                        it.next();
                        if d.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}

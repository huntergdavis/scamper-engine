//! Graphics backends: turn a rendered RGBA framebuffer into terminal output.
//!
//! The game renders its scene into a `Framebuffer` with no knowledge of how it
//! reaches the screen — that is this module's job. A `Backend` is fully decoupled
//! behind a trait so backends are swappable at runtime (press Tab in-game):
//!
//! - [`KittyBackend`] transmits the framebuffer as a scaled, double-buffered
//!   Kitty graphics image (pixel-perfect, terminal must speak the protocol).
//! - [`TextBackend`] samples the framebuffer into Unicode half-block cells
//!   (works in any terminal). See `text.rs`.
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

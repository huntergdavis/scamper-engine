//! tile-lab — a standalone tool (built on the Scamper engine) for viewing the
//! level tile set in every render backend, the tile counterpart to `sprite-lab`.
//! It fills the stage with one tile kind at a time so you can see how it reads as
//! a surface from mono ASCII up to Kitty pixels, and across themes.
//!
//! Controls: space → next tile kind, t → next theme, Tab → next graphics backend,
//! q/Esc → quit.

use scamper::backend::{AsciiBackend, Backend, KittyBackend, MonoBackend, TextBackend};
use scamper::framebuffer::Framebuffer;
use scamper::input::{Input, K_ESC, K_Q, K_SPACE, K_T, K_TAB};
use scamper::level::art::{self, Theme, TILE};
use scamper::terminal;
use scamper::time::{now_ns, sleep_until_ns, NS_PER_SEC};
use std::io::Write;

/// Stage geometry: a framebuffer + the terminal cell area (last row left for the
/// status line). Mirrors sprite-lab's sizing.
struct Stage {
    fb_w: usize,
    fb_h: usize,
    cols: u16,
    play_rows: u16,
}

fn build_stage(ws: terminal::WinSize) -> Stage {
    let cols = ws.cols.max(20);
    let rows = ws.rows.max(8);
    let (xpix, ypix) = if ws.xpix > 0 && ws.ypix > 0 {
        (ws.xpix as f64, ws.ypix as f64)
    } else {
        (cols as f64 * 8.0, rows as f64 * 16.0)
    };
    let cell_h = ypix / rows as f64;
    let play_h = (ypix - cell_h).max(cell_h); // reserve the status row
    let scale = (xpix.max(play_h) / 320.0).max(1.0);
    Stage {
        fb_w: (xpix / scale).round().max(16.0) as usize,
        fb_h: (play_h / scale).round().max(16.0) as usize,
        cols,
        play_rows: rows - 1,
    }
}

fn next_backend(name: &str) -> Box<dyn Backend> {
    match name {
        "kitty" => Box::new(TextBackend::new()),
        "text" => Box::new(AsciiBackend::new()),
        "ascii" => Box::new(MonoBackend::new()),
        _ => Box::new(KittyBackend::new()),
    }
}

fn clear_screen(out: &mut Vec<u8>, backend: &mut Box<dyn Backend>) {
    out.clear();
    backend.teardown(out);
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(out);
    let _ = o.write_all(b"\x1b[2J");
    let _ = o.flush();
}

fn main() {
    let guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("tile-lab needs an interactive terminal: {e}");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();
    let mut input = Input::new(kitty_kbd);

    let mut stage = build_stage(terminal::query_winsize());
    let mut fb = Framebuffer::new(stage.fb_w, stage.fb_h);
    let mut backend: Box<dyn Backend> = Box::new(AsciiBackend::new());

    let mut ki = 0usize; // tile kind
    let mut ti = 0usize; // theme
    let mut out: Vec<u8> = Vec::new();
    let mut status = String::new();
    let mut dirty = true; // only re-encode when something changed (kitty is heavy)
    use std::fmt::Write as _;

    loop {
        if terminal::quit_requested() || input.quit {
            break;
        }
        input.poll();
        if input.pressed(K_Q) || input.pressed(K_ESC) {
            break;
        }
        if input.pressed(K_TAB) {
            clear_screen(&mut out, &mut backend);
            backend = next_backend(backend.name());
            dirty = true;
        }
        if input.pressed(K_SPACE) {
            ki = (ki + 1) % art::KINDS.len();
            dirty = true;
        }
        if input.pressed(K_T) {
            ti = (ti + 1) % Theme::ALL.len();
            dirty = true;
        }
        if terminal::take_resize() {
            stage = build_stage(terminal::query_winsize());
            fb.resize(stage.fb_w, stage.fb_h);
            clear_screen(&mut out, &mut backend);
            dirty = true;
        }

        if dirty {
            let kind = art::KINDS[ki];
            let theme = Theme::ALL[ti];
            let pal = art::palette(theme);
            // Fill the stage with the current kind so it reads as a surface; the
            // theme sky shows through any non-solid pixels (platform/deco/hidden).
            fb.clear(pal.sky);
            let t = TILE;
            let mut y = 0;
            while y < fb.height as i32 {
                let mut x = 0;
                while x < fb.width as i32 {
                    art::draw_tile(&mut fb, x, y, kind, &pal);
                    x += t;
                }
                y += t;
            }
            backend.present(&mut out, &fb, stage.cols, stage.play_rows, true, &[]);

            status.clear();
            let _ = write!(
                status,
                "\x1b[{};1H\x1b[2K\x1b[2mTILE LAB  {}  ·  theme:{}  ·  gfx:{}   \x1b[4mTab\x1b[24m gfx  space tile  t theme  q quit\x1b[0m",
                stage.play_rows + 1, kind.as_str(), theme.name(), backend.name()
            );
            {
                let mut o = std::io::stdout().lock();
                let _ = o.write_all(&out);
                let _ = o.write_all(status.as_bytes());
                let _ = o.flush();
            }
            dirty = false;
        }

        sleep_until_ns(now_ns() + NS_PER_SEC / 30, 1_000_000);
    }
    drop(guard);
    eprintln!("tile-lab: bye.");
}

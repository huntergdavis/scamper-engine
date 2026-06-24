//! sprite-lab — a standalone tool (built on the Scamper engine) for viewing
//! sprite animations in every render backend. As the engine's demo games gain
//! sprites, register their animation sets here and the lab plays them all back.
//!
//! Controls: space → next animation, Tab → next graphics backend,
//! s → next sprite set, q/Esc → quit.

use scamper::backend::{AsciiBackend, Backend, KittyBackend, MonoBackend, Overlay, TextBackend};
use scamper::framebuffer::{Framebuffer, Rgba};
use scamper::input::{Input, K_ESC, K_Q, K_S, K_SPACE, K_TAB};
use scamper::munchii::{self, Anim, ALL as MUNCHII};
use scamper::terminal;
use scamper::time::{now_ns, sleep_until_ns, NS_PER_SEC};
use std::io::Write;

/// A named collection of animations (one character's sprite sheet).
struct SpriteSet {
    name: &'static str,
    anims: &'static [Anim],
}

/// Everything the lab can show. Append sets here as the engine grows.
const SETS: &[SpriteSet] = &[SpriteSet { name: "munchii", anims: MUNCHII }];

const BG: Rgba = Rgba::rgb(18, 18, 26);

/// Stage geometry: a dark framebuffer + the terminal cell area (last row left
/// for the status line). Mirrors the game's sizing, without tiles.
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

/// Rasterize a sprite frame into the framebuffer (pixel tiers): each glyph a
/// cell-sized block in its beagle color, top-left at (`lx`,`ly`) px.
fn rasterize(fb: &mut Framebuffer, frame: &[&str], lx: f64, ly: f64, cpw: f64, cph: f64) {
    let bw = cpw.ceil() as i32;
    let bh = cph.ceil() as i32;
    for (gr, line) in frame.iter().enumerate() {
        for (gc, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let (r, g, b) = munchii::beagle_rgb(ch);
            let px = (lx + gc as f64 * cpw).round() as i32;
            let py = (ly + gr as f64 * cph).round() as i32;
            fb.fill_rect(px, py, bw, bh, Rgba::rgb(r, g, b));
        }
    }
}

fn main() {
    let guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("sprite-lab needs an interactive terminal: {e}");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();
    let mut input = Input::new(kitty_kbd);

    let mut stage = build_stage(terminal::query_winsize());
    let mut fb = Framebuffer::new(stage.fb_w, stage.fb_h);
    let mut backend: Box<dyn Backend> = Box::new(AsciiBackend::new());
    let mut full = true;

    let mut si = 0usize; // sprite set
    let mut ai = 0usize; // animation
    let mut fi = 0usize; // frame
    let mut last = now_ns();
    let mut out: Vec<u8> = Vec::new();
    let mut status = String::new();
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
            out.clear();
            backend.teardown(&mut out);
            {
                let mut o = std::io::stdout().lock();
                let _ = o.write_all(&out);
                let _ = o.write_all(b"\x1b[2J");
                let _ = o.flush();
            }
            backend = next_backend(backend.name());
            full = true;
        }
        if input.pressed(K_S) && SETS.len() > 1 {
            si = (si + 1) % SETS.len();
            ai = 0;
            fi = 0;
            last = now_ns();
        }
        if input.pressed(K_SPACE) {
            ai = (ai + 1) % SETS[si].anims.len();
            fi = 0;
            last = now_ns();
        }
        if terminal::take_resize() {
            stage = build_stage(terminal::query_winsize());
            fb.resize(stage.fb_w, stage.fb_h);
            backend.teardown(&mut out);
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(b"\x1b[2J");
            let _ = o.flush();
            full = true;
        }

        let set = &SETS[si];
        let anim = &set.anims[ai];
        let now = now_ns();
        if anim.frames.is_empty() {
            sleep_until_ns(now_ns() + NS_PER_SEC / 60, 1_000_000);
            continue; // a frameless anim has nothing to draw
        }
        if anim.frames.len() > 1 && now - last >= NS_PER_SEC / anim.fps.max(1) as u64 {
            fi = (fi + 1) % anim.frames.len();
            last = now;
        }
        let frame = anim.frames[fi.min(anim.frames.len() - 1)];

        // Center the sprite in the stage.
        let cols = stage.cols as usize;
        let prows = stage.play_rows as usize;
        let sw = frame.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let sh = frame.len();
        let col0 = (cols.saturating_sub(sw) / 2) as i32;
        let row0 = (prows.saturating_sub(sh) / 2) as i32;

        fb.clear(BG);
        if backend.draws_overlay() {
            let lines: Vec<String> = frame.iter().map(|s| s.to_string()).collect();
            let ov = [Overlay { lines: &lines, col: col0, row: row0, tint: None, palette: None, z: 0 }];
            backend.present(&mut out, &fb, stage.cols, stage.play_rows, full, &ov);
        } else {
            let cpw = stage.fb_w as f64 / cols.max(1) as f64;
            let cph = stage.fb_h as f64 / prows.max(1) as f64;
            rasterize(&mut fb, frame, col0 as f64 * cpw, row0 as f64 * cph, cpw, cph);
            backend.present(&mut out, &fb, stage.cols, stage.play_rows, full, &[]);
        }
        full = false;

        // Status line (last row).
        status.clear();
        let _ = write!(
            status,
            "\x1b[{};1H\x1b[2K\x1b[2mSPRITE LAB  {}/{}  ({}f @ {}fps)  gfx:{}   \x1b[4mTab\x1b[24m gfx  space anim  q quit\x1b[0m",
            stage.play_rows + 1, set.name, anim.name, anim.frames.len(), anim.fps, backend.name()
        );
        {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(status.as_bytes());
            let _ = o.flush();
        }
        sleep_until_ns(now_ns() + NS_PER_SEC / 60, 1_000_000);
    }
    drop(guard);
    eprintln!("sprite-lab: bye.");
}

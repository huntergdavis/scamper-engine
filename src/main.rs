//! `scamp` — the game binary: a sandbox platformer level driven by keyboard,
//! rendered to a Kitty terminal. Also a headless `verify` mode that runs scripted
//! scenarios and dumps PNGs (for development on a box without a Kitty terminal).

use scamper::backend::{AsciiBackend, Backend, KittyBackend, MonoBackend, Overlay, TextBackend};
use scamper::effects::{self, Effects};
use scamper::munchii;
use scamper::framebuffer::{Framebuffer, Rgba};
use scamper::input::{Input, K_ESC, K_HELP, K_N, K_Q, K_TAB, K_Y};
use scamper::math::Vec2;
use scamper::player::{FeelParams, Player, State};
use scamper::time::{now_ns, sleep_until_ns, NS_PER_SEC};
use scamper::world::{TileMap, TILE};
use scamper::{dlog, kitty, terminal};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `--debug` (anywhere) turns on file logging to scamp.log (override path with
    // SCAMP_LOG). Kept on by default during development via run.sh.
    let debug = args.iter().any(|a| a == "--debug");
    let log_path = std::env::var("SCAMP_LOG").unwrap_or_else(|_| "scamp.log".into());
    scamper::dbg::init(debug, &log_path);
    dlog!("scamp start: args={:?} TERM={:?}", &args[1..], std::env::var("TERM").ok());

    // The first non-flag argument is the subcommand.
    let cmd = args.iter().skip(1).find(|a| !a.starts_with("--")).map(|s| s.as_str());
    match cmd {
        Some("verify") => {
            // dir = first non-flag arg after the subcommand
            let dir = args
                .iter()
                .skip(1)
                .filter(|a| !a.starts_with("--"))
                .nth(1)
                .map(|s| s.as_str())
                .unwrap_or(".");
            run_verify(dir);
        }
        Some("info") => {
            let ws = terminal::query_winsize();
            println!("winsize: {ws:?}");
        }
        Some("gfxtest") => run_gfxtest(),
        Some("shot") => run_shot(),
        _ => run_live(),
    }
}

// ---------------------------------------------------------------------------
// Graphics probe — isolate "can this terminal draw a Kitty image" from the loop
// ---------------------------------------------------------------------------

/// Display one static image (a bordered box with a red square + color bars) and
/// wait for a key. Prints winsize / TERM / keyboard-protocol support AFTER
/// teardown so it's readable on the normal screen even if the image never shows.
fn run_gfxtest() {
    let ws = terminal::query_winsize();
    let term = std::env::var("TERM").unwrap_or_else(|_| "<unset>".into());

    let guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("gfxtest needs an interactive terminal: {e}");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();

    // A fixed, modest image — independent of the reported pixel size.
    let (w, h) = (320usize, 200usize);
    let mut fb = Framebuffer::new(w, h);
    fb.clear(Rgba::rgb(30, 32, 46));
    fb.stroke_rect(0, 0, w as i32, h as i32, Rgba::rgb(255, 255, 255));
    // RGB bars so a partial/odd render is still diagnosable.
    fb.fill_rect(20, 20, 60, 40, Rgba::rgb(230, 60, 60));
    fb.fill_rect(90, 20, 60, 40, Rgba::rgb(60, 220, 80));
    fb.fill_rect(160, 20, 60, 40, Rgba::rgb(80, 120, 240));
    // center red square (the "sprite")
    fb.fill_rect(w as i32 / 2 - 24, h as i32 / 2 - 24, 48, 48, Rgba::rgb(244, 180, 60));
    fb.stroke_rect(w as i32 / 2 - 24, h as i32 / 2 - 24, 48, 48, Rgba::rgb(255, 245, 210));

    let mut out = Vec::new();
    let mut b64 = Vec::new();
    kitty::present_rgba(&mut out, kitty::BUF_A, w, h, 0, 0, &fb.px, &mut b64);
    dlog!("gfxtest: image {w}x{h}px, encoded {} bytes (b64 {}), winsize={ws:?}", out.len(), b64.len());
    {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(&out);
        // status hint on the bottom-ish; keep it simple (row 25).
        let _ = o.write_all(b"\x1b[25;1H\x1b[2Kgfxtest: see a 320x200 box w/ RGB bars + orange square? press q to quit.");
        let _ = o.flush();
    }

    let mut input = Input::new(kitty_kbd);
    loop {
        if terminal::quit_requested() {
            break;
        }
        input.poll();
        if input.quit {
            break;
        }
        sleep_until_ns(now_ns() + NS_PER_SEC / 30, 1_000_000);
    }
    drop(guard);

    println!("gfxtest done.");
    println!("  TERM           = {term}");
    println!("  winsize        = {ws:?}");
    println!("  kitty keyboard = {kitty_kbd}");
    if ws.xpix == 0 || ws.ypix == 0 {
        println!("  NOTE: terminal did not report pixel size (xpix/ypix=0) — the game");
        println!("        falls back to an 8x16 cell guess, which can mis-size the arena.");
    }
}

// ---------------------------------------------------------------------------
// Screenshot — render a compact arena + Munchii to plain text (for the README)
// ---------------------------------------------------------------------------

fn run_shot() {
    // A compact, README-friendly window size.
    let ws = terminal::WinSize { cols: 54, rows: 16, xpix: 540, ypix: 320 };
    let arena = build_arena(ws);
    let mut fb = Framebuffer::new(arena.fb_w, arena.fb_h);
    let mut player = Player::new(arena.map.spawn.0, arena.map.spawn.1);
    fit_player_to_munchii(&mut player, &arena);

    // Settle on the floor, then stand him in the middle facing right.
    let fp = FeelParams::default();
    for _ in 0..30 {
        player.step(&arena.map, 1.0 / 60.0, 0.0, false, false, false, &fp);
    }
    player.pos.x = (arena.map.px_w() - player.w) / 2.0;

    render_arena(&mut fb, &arena.map);
    let cols = arena.cols as usize;
    let disp_rows = arena.rows.saturating_sub(1) as usize;

    // Munchii's idle pose, aligned to his hitbox.
    let frame = munchii::anim("idle").frames[0];
    let lines: Vec<String> = frame.iter().map(|s| s.to_string()).collect();
    let fw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as i32;
    let box_left = (player.pos.x / arena.fb_w as f64 * arena.cols as f64).round() as i32;
    let box_top = (player.pos.y / arena.fb_h as f64 * disp_rows as f64).round() as i32;
    let ov = [Overlay {
        lines: &lines,
        col: box_left + (munchii::W as i32 - fw) / 2,
        row: box_top,
        tint: None,
        z: 0,
    }];
    print!("{}", scamper::backend::mono_text(&fb, cols, disp_rows, &ov));
}

// ---------------------------------------------------------------------------
// Levels
// ---------------------------------------------------------------------------

fn build_sandbox() -> TileMap {
    let (w, h) = (56usize, 22usize);
    let mut m = TileMap::new(w, h);
    // borders
    for x in 0..w {
        m.set(x, 0, true);
        m.set(x, h - 1, true);
    }
    for y in 0..h {
        m.set(0, y, true);
        m.set(w - 1, y, true);
    }
    // 2-thick ground with a pit gap
    for x in 1..w - 1 {
        m.set(x, h - 2, true);
        m.set(x, h - 3, true);
    }
    for x in 24..31 {
        m.set(x, h - 2, false);
        m.set(x, h - 3, false);
    }
    // low platform (single jump)
    for x in 8..14 {
        m.set(x, h - 6, true);
    }
    // higher platform (double jump)
    for x in 16..21 {
        m.set(x, h - 10, true);
    }
    // high ledge on the left
    for x in 3..8 {
        m.set(x, h - 12, true);
    }
    // wall-jump shaft: two pillars with a 2-tile gap between (cols 39,40 open)
    for y in (h - 13)..(h - 1) {
        m.set(38, y, true);
        m.set(41, y, true);
    }
    // a ceiling block to test head bonk / corner
    for x in 30..36 {
        m.set(x, h - 9, true);
    }
    m.spawn = (3.0 * TILE, (h as f64 - 5.0) * TILE);
    m
}

/// A simple tall wall on the right for isolating wall-slide / wall-jump.
fn wall_test_map() -> TileMap {
    let (w, h) = (20usize, 20usize);
    let mut m = TileMap::new(w, h);
    for x in 0..w {
        m.set(x, h - 1, true);
        m.set(x, 0, true);
    }
    for y in 0..h {
        m.set(0, y, true);
    }
    // tall wall on the right
    for y in 1..h {
        m.set(w - 1, y, true);
    }
    // ground
    for x in 1..w - 1 {
        m.set(x, h - 2, true);
    }
    m.spawn = ((w as f64 - 3.0) * TILE, 2.0 * TILE);
    m
}

/// The engine test arena: a solid box hugging the terminal window (minus the
/// bottom status row), sized to whatever aspect ratio is open. Floor, ceiling
/// and both walls are a one-tile-thick border so every movement function — run,
/// jump, double-jump, wall-slide, wall-jump, fast-fall — is reachable inside it.
struct Arena {
    map: TileMap,
    fb_w: usize,
    fb_h: usize,
    rows: u16, // terminal rows (status is drawn on the last one)
    cols: u16,
}

/// Cap on the larger side of the *internal* render image, in pixels. The image
/// is transmitted at this resolution and the terminal scales it up to fill the
/// window — so per-frame bandwidth is bounded regardless of window size. (A
/// full-window native image is megabytes/frame; this keeps it well under 1 MB.)
const MAX_INTERNAL_DIM: f64 = 320.0;

fn build_arena(ws: terminal::WinSize) -> Arena {
    let cols = ws.cols.max(20);
    let rows = ws.rows.max(6);
    // Pixel size, with a fallback for terminals that don't report it via TIOCGWINSZ.
    let (xpix, ypix) = if ws.xpix > 0 && ws.ypix > 0 {
        (ws.xpix as f64, ws.ypix as f64)
    } else {
        (cols as f64 * 8.0, rows as f64 * 16.0)
    };
    let cell_h = ypix / rows as f64;
    // Play area in window pixels: full width, minus the reserved bottom status row.
    let play_w = xpix;
    let play_h = (ypix - cell_h).max(cell_h);
    // Downscale to a modest internal resolution (aspect preserved), then snap to
    // whole tiles. The terminal upscales the result back to the play area.
    let scale = (play_w.max(play_h) / MAX_INTERNAL_DIM).max(1.0);
    let tiles_w = ((play_w / scale / TILE).round() as usize).max(6);
    let tiles_h = ((play_h / scale / TILE).round() as usize).max(6);

    let mut map = TileMap::new(tiles_w, tiles_h);
    for x in 0..tiles_w {
        map.set(x, 0, true);
        map.set(x, tiles_h - 1, true);
    }
    for y in 0..tiles_h {
        map.set(0, y, true);
        map.set(tiles_w - 1, y, true);
    }
    // Spawn on the floor, a little in from the left wall.
    map.spawn = (2.0 * TILE, (tiles_h as f64 - 2.0) * TILE);

    Arena {
        map,
        fb_w: tiles_w * TILE as usize,
        fb_h: tiles_h * TILE as usize,
        rows,
        cols,
    }
}

/// Keep the player inside the open interior of the box (between the border
/// tiles). Used after a resize so a shrunk window never traps it in a wall.
fn clamp_into_arena(p: &mut Player, arena: &Arena) {
    let min_x = TILE;
    let min_y = TILE;
    let max_x = (arena.map.px_w() - TILE - p.w).max(min_x);
    let max_y = (arena.map.px_h() - TILE - p.h).max(min_y);
    let cx = p.pos.x.clamp(min_x, max_x);
    let cy = p.pos.y.clamp(min_y, max_y);
    if cx != p.pos.x || cy != p.pos.y {
        p.pos.x = cx;
        p.pos.y = cy;
        p.vel = Vec2::ZERO;
    }
}

/// Munchii's on-screen footprint in framebuffer pixels (his sprite-cell size
/// mapped through the current arena's cell↔pixel scale). The player's collision
/// box is set to this so the hitbox matches the drawn dog at every backend.
fn munchii_box(arena: &Arena) -> (f64, f64) {
    let disp_rows = arena.rows.saturating_sub(1).max(1) as f64;
    let w = munchii::W as f64 / arena.cols.max(1) as f64 * arena.fb_w as f64;
    let h = munchii::H as f64 / disp_rows * arena.fb_h as f64;
    // Never let the box exceed the open interior between the border tiles —
    // otherwise on a tiny window it embeds in the walls and the sweep, which
    // can't depenetrate a pre-existing overlap, freezes him in place.
    let iw = (arena.map.px_w() - 2.0 * TILE).max(8.0);
    let ih = (arena.map.px_h() - 2.0 * TILE).max(8.0);
    (w.clamp(8.0, iw), h.clamp(8.0, ih))
}

/// Resize the player's hitbox to Munchii's footprint and reseat it onto the
/// floor / inside the box.
fn fit_player_to_munchii(p: &mut Player, arena: &Arena) {
    let (w, h) = munchii_box(arena);
    p.w = w;
    p.h = h;
    clamp_into_arena(p, arena);
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

const SKY: Rgba = Rgba::rgb(20, 22, 34);
const TILE_FILL: Rgba = Rgba::rgb(58, 66, 92);
const TILE_TOP: Rgba = Rgba::rgb(90, 102, 140);

fn state_color(s: State) -> Rgba {
    match s {
        State::Grounded => Rgba::rgb(240, 200, 80),
        State::Airborne => Rgba::rgb(244, 140, 60),
        State::WallSliding => Rgba::rgb(90, 205, 230),
    }
}

/// Render the tile map (no camera, no player) into the framebuffer.
fn render_arena(fb: &mut Framebuffer, map: &TileMap) {
    fb.clear(SKY);
    let t = TILE as i32;
    for ty in 0..map.h {
        for tx in 0..map.w {
            if map.is_solid(tx as i32, ty as i32) {
                let x = tx as i32 * t;
                let y = ty as i32 * t;
                fb.fill_rect(x, y, t, t, TILE_FILL);
                // light top edge if open above (depth cue)
                if !map.is_solid(tx as i32, ty as i32 - 1) {
                    fb.fill_rect(x, y, t, 2, TILE_TOP);
                }
            }
        }
    }
}

/// Draw the player as a colored box at a *visual* size `vis_w × vis_h` (which may
/// exceed the collision box), centered on the hitbox. Used by the pixel backends
/// (kitty/text); the character backends draw the Munchii sprite instead.
fn draw_player(fb: &mut Framebuffer, rpos: Vec2, player: &Player, vis_w: f64, vis_h: f64) {
    let cx = rpos.x + player.w / 2.0;
    let cy = rpos.y + player.h / 2.0;
    let pw = vis_w.round().max(2.0) as i32;
    let ph = vis_h.round().max(2.0) as i32;
    let px = (cx - vis_w / 2.0).round() as i32;
    let py = (cy - vis_h / 2.0).round() as i32;
    let col = state_color(player.state);
    fb.fill_rect(px, py, pw, ph, col);
    fb.stroke_rect(px, py, pw, ph, Rgba::rgb(255, 245, 210));
    // facing "eye"
    let eye_x = if player.facing >= 0 { px + pw - 5 } else { px + 2 };
    fb.fill_rect(eye_x, py + ph / 4, 3, 3, Rgba::rgb(20, 20, 20));
    // velocity vector (debug overlay)
    let ccx = px + pw / 2;
    let ccy = py + ph / 2;
    let vscale = 0.06;
    fb.line(
        ccx,
        ccy,
        ccx + (player.vel.x * vscale) as i32,
        ccy + (player.vel.y * vscale) as i32,
        Rgba::rgb(255, 80, 80),
    );
}

/// Convenience used by the headless verify harness: arena + player at hitbox size.
fn render(fb: &mut Framebuffer, map: &TileMap, rpos: Vec2, player: &Player) {
    render_arena(fb, map);
    draw_player(fb, rpos, player, player.w, player.h);
}

/// Rasterize Munchii's sprite into the framebuffer (the pixel tiers' version of
/// the character): each glyph becomes a cell-sized block in its beagle color,
/// top-left at (`lx`,`ly`) px. Matches what mono/ascii stamp as the overlay.
fn draw_sprite_pixels(fb: &mut Framebuffer, lines: &[String], lx: f64, ly: f64, cpw: f64, cph: f64) {
    for (gr, line) in lines.iter().enumerate() {
        for (gc, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let (r, g, b) = munchii::beagle_rgb(ch);
            cell_block(fb, lx, ly, gc, gr, cpw, cph, Rgba::rgb(r, g, b));
        }
    }
}

/// Fill the block for glyph (gc, gr) by snapping to cell *boundaries*, so blocks
/// tile exactly (no `ceil` inflation): N glyphs span exactly N·cpw px, matching
/// the character tiers' N cells.
#[inline]
fn cell_block(fb: &mut Framebuffer, lx: f64, ly: f64, gc: usize, gr: usize, cpw: f64, cph: f64, col: Rgba) {
    let x0 = (lx + gc as f64 * cpw).floor() as i32;
    let x1 = (lx + (gc as f64 + 1.0) * cpw).floor() as i32;
    let y0 = (ly + gr as f64 * cph).floor() as i32;
    let y1 = (ly + (gr as f64 + 1.0) * cph).floor() as i32;
    fb.fill_rect(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1), col);
}

/// Rasterize an effect clip into the framebuffer (the pixel tiers' version of an
/// effect): each non-space glyph becomes a cell-sized block in the effect tint,
/// so it matches the character tiers' look. `ax`/`ay` = clip anchor (center-x,
/// top-y) in framebuffer px; `cpw`/`cph` = one cell's pixel size.
fn draw_effect_pixels(fb: &mut Framebuffer, frame: &[&str], tint: (u8, u8, u8), ax: f64, ay: f64, cpw: f64, cph: f64) {
    let w_cells = frame.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let left = ax - w_cells as f64 * cpw / 2.0;
    let col = Rgba::rgb(tint.0, tint.1, tint.2);
    for (gr, line) in frame.iter().enumerate() {
        for (gc, ch) in line.chars().enumerate() {
            if ch != ' ' {
                cell_block(fb, left, ay, gc, gr, cpw, cph, col);
            }
        }
    }
}

/// Mirror one sprite row horizontally (for a left-facing Munchii), swapping the
/// directional glyphs so the drawing stays coherent.
fn flip_line(s: &str) -> String {
    s.chars()
        .rev()
        .map(|c| match c {
            '(' => ')',
            ')' => '(',
            '/' => '\\',
            '\\' => '/',
            '<' => '>',
            '>' => '<',
            other => other,
        })
        .collect()
}

/// Munchii's looping pose for his current movement state. The double-jump burst
/// is handled separately (it's a one-shot, not a loop).
fn pose_for(player: &Player, down_held: bool) -> &'static str {
    if player.state == State::WallSliding {
        "wall-slide"
    } else if !player.grounded {
        "jump"
    } else if down_held {
        "crawl"
    } else if player.vel.x.abs() > 8.0 {
        "walk"
    } else {
        "idle"
    }
}

// ---------------------------------------------------------------------------
// Status line (bottom terminal row): help hint + backend + live engine readout
// ---------------------------------------------------------------------------

fn state_letter(s: State) -> &'static str {
    match s {
        State::Grounded => "GROUND",
        State::Airborne => "AIR",
        State::WallSliding => "WALL",
    }
}

/// Build the bottom status row. Positions to the last row, clears it, and writes
/// a single line (truncated to terminal width so it never wraps/scrolls). The
/// leading `h` (the help affordance) is underlined; full controls + quit live in
/// the help menu.
fn render_status(buf: &mut String, p: &Player, score: u32, fps: f64, backend: &str, rows: u16, cols: u16) {
    use std::fmt::Write;
    let mut plain = String::new();
    let _ = write!(
        plain,
        "h Help  |  Tab gfx:{backend}  |  Score {score}  |  {}  vx {:>4.0} vy {:>4.0}  |  {fps:>3.0} fps",
        state_letter(p.state),
        p.vel.x,
        p.vel.y,
    );
    // Truncate to fit (leave 1 col of slack so the cursor never forces a wrap).
    let maxw = (cols as usize).saturating_sub(1);
    if plain.len() > maxw {
        plain.truncate(maxw);
    }

    buf.clear();
    let _ = write!(buf, "\x1b[{rows};1H\x1b[2K\x1b[2m", rows = rows); // go to last row, clear, dim
    if let Some(rest) = plain.strip_prefix('h') {
        buf.push_str("\x1b[4mh\x1b[24m"); // underlined help affordance
        buf.push_str(rest);
    } else {
        buf.push_str(&plain);
    }
    buf.push_str("\x1b[0m");
}

// ---------------------------------------------------------------------------
// Modal UI: play, help menu, quit confirmation
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Ui {
    Play,
    Help,
    ConfirmQuit,
}

/// Quit gate: a reverse-video prompt on the status row. Quitting is gated so a
/// stray Q/Esc can't drop you out mid-play.
fn render_quit_prompt(buf: &mut String, rows: u16, cols: u16) {
    use std::fmt::Write;
    let plain = "Really quit?   Y = yes    N / Esc = keep playing";
    // Reserve 1 col of slack + the 2 padding spaces in the reverse-video bar.
    let maxw = (cols as usize).saturating_sub(3);
    let shown = if plain.len() > maxw { &plain[..maxw] } else { plain };
    buf.clear();
    let _ = write!(buf, "\x1b[{rows};1H\x1b[2K\x1b[7m {shown} \x1b[0m");
}

/// Write one help line at (row, col 3), clearing the rest of the line first so
/// shorter content (e.g. a changed backend name) leaves no trailing junk.
fn hline(out: &mut Vec<u8>, row: u16, s: &str) {
    use std::io::Write;
    let _ = write!(out, "\x1b[{row};3H\x1b[K{s}");
}

/// Full-screen help/controls + graphics-backend explainer. Drawn as plain text
/// so it works under either backend (the live image is torn down on entry).
fn render_help(out: &mut Vec<u8>, active_backend: &str) {
    out.clear();
    let mut r = 2u16;
    hline(out, r, "\x1b[1mSCAMPER — controls & graphics\x1b[0m");
    r += 2;
    hline(out, r, "Move              A / D   or   \u{2190} / \u{2192}");
    r += 1;
    hline(out, r, "Jump (hold=higher)  Space / Z / K / W / \u{2191}");
    r += 1;
    hline(out, r, "Double jump       jump again in mid-air");
    r += 1;
    hline(out, r, "Wall slide / jump push into a wall, then jump");
    r += 1;
    hline(out, r, "Fast-fall         S / \u{2193}");
    r += 2;
    hline(out, r, &format!("Tab               switch graphics backend   [now: {active_backend}]"));
    r += 1;
    hline(out, r, "h                 toggle this help");
    r += 1;
    hline(out, r, "Q / Esc           quit (confirm with Y / N)");
    r += 2;
    hline(out, r, "\x1b[1mGraphics backends\x1b[0m  (Tab cycles)");
    r += 1;
    hline(out, r, "  kitty   pixel image via the Kitty graphics protocol (sharp)");
    r += 1;
    hline(out, r, "  text    Unicode half-block cells, color (works anywhere)");
    r += 1;
    hline(out, r, "  ascii   colored ASCII glyphs (retro art)");
    r += 1;
    hline(out, r, "  mono    plain black & white ASCII (bare minimum)");
    r += 2;
    hline(out, r, "\x1b[2mpress h or Esc to resume\x1b[0m");
}

// ---------------------------------------------------------------------------
// Live loop — the engine test app: a box arena that fills the terminal window.
// ---------------------------------------------------------------------------

fn run_live() {
    let guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("scamp needs an interactive terminal (Kitty/Ghostty/foot). ({e})");
            eprintln!("Try: run it directly in a Kitty terminal, or `scamp verify <dir>` for a headless render dump.");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();

    let fp = FeelParams::default();
    let ws0 = terminal::query_winsize();
    let mut arena = build_arena(ws0);
    let mut fb = Framebuffer::new(arena.fb_w, arena.fb_h);
    let mut player = Player::new(arena.map.spawn.0, arena.map.spawn.1);
    fit_player_to_munchii(&mut player, &arena);
    let mut input = Input::new(kitty_kbd);
    dlog!(
        "live: kitty_kbd={kitty_kbd} winsize={ws0:?} -> arena {}x{} tiles, internal image {}x{}px scaled across {}x{} cells, spawn=({:.0},{:.0})",
        arena.map.w, arena.map.h, arena.fb_w, arena.fb_h, arena.cols, arena.rows.saturating_sub(1), arena.map.spawn.0, arena.map.spawn.1
    );

    let mut out: Vec<u8> = Vec::new();
    let mut status = String::new();
    let score: u32 = 0;
    let mut fps = 60.0_f64;
    let mut backend: Box<dyn Backend> = Box::new(KittyBackend::new());
    let mut full_redraw = true; // force a complete repaint after switch/resize

    let sim_dt = 1.0 / 60.0;
    let sim_dt_ns = NS_PER_SEC / 60;
    let spin_margin = 1_000_000u64; // 1ms
    let mut acc: u64 = 0;
    let mut prev_t = now_ns();
    let mut next = now_ns();
    let mut prev_pos = player.pos;
    let mut pending_jump = false; // latch a press until a sim substep consumes it
    let mut frame: u64 = 0;
    let mut fx = Effects::new();
    let mut was_double = false; // rising edge of did_double = the double-jump fires
    let mut was_grounded = true; // falling edge of grounded = a landing
    let mut last_spark_ns: u64 = 0; // throttles the continuous wall-slide sparks

    let mut ui = Ui::Play;
    // Switch the active backend (kitty <-> text), clearing the old one's output.
    let switch_backend = |backend: &mut Box<dyn Backend>| {
        let mut o2: Vec<u8> = Vec::new();
        backend.teardown(&mut o2);
        {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&o2);
            let _ = o.write_all(b"\x1b[2J");
            let _ = o.flush();
        }
        // Cycle: kitty -> text -> ascii -> mono -> kitty.
        *backend = match backend.name() {
            "kitty" => Box::new(TextBackend::new()) as Box<dyn Backend>,
            "text" => Box::new(AsciiBackend::new()) as Box<dyn Backend>,
            "ascii" => Box::new(MonoBackend::new()) as Box<dyn Backend>,
            _ => Box::new(KittyBackend::new()) as Box<dyn Backend>,
        };
        dlog!("backend -> {}", backend.name());
    };

    loop {
        if terminal::quit_requested() || input.quit {
            break; // external signal / Ctrl-C — the hard escape hatch (no gate)
        }
        input.poll();
        if input.quit {
            break;
        }

        // --- modal / UI transitions ---
        match ui {
            Ui::Play => {
                if input.pressed(K_HELP) {
                    ui = Ui::Help;
                    // Tear down the live image so help text isn't hidden behind it.
                    out.clear();
                    backend.teardown(&mut out);
                    let mut o = std::io::stdout().lock();
                    let _ = o.write_all(&out);
                    let _ = o.write_all(b"\x1b[2J");
                    let _ = o.flush();
                } else if input.pressed(K_Q) || input.pressed(K_ESC) {
                    ui = Ui::ConfirmQuit;
                } else if input.pressed(K_TAB) {
                    switch_backend(&mut backend);
                    full_redraw = true;
                }
            }
            Ui::ConfirmQuit => {
                if input.pressed(K_Y) {
                    break;
                }
                if input.pressed(K_N) || input.pressed(K_ESC) {
                    ui = Ui::Play;
                    full_redraw = true;
                }
            }
            Ui::Help => {
                if input.pressed(K_HELP) || input.pressed(K_ESC) {
                    ui = Ui::Play;
                    full_redraw = true;
                } else if input.pressed(K_TAB) {
                    switch_backend(&mut backend); // stays in help; redrawn below
                }
            }
        }

        // Rebuild the arena to the new window size, keeping the player in bounds.
        if terminal::take_resize() {
            let ws = terminal::query_winsize();
            arena = build_arena(ws);
            fb.resize(arena.fb_w, arena.fb_h);
            dlog!("resize: winsize={ws:?} -> arena {}x{} tiles, image {}x{}px", arena.map.w, arena.map.h, arena.fb_w, arena.fb_h);
            // Footprint depends on the window, so refit Munchii's hitbox to the
            // new arena and reseat him inside it (also rescues a shrink that would
            // otherwise trap him in a wall).
            fit_player_to_munchii(&mut player, &arena);
            prev_pos = player.pos;
            // Dimensions changed: clear the backend's artifacts + screen, then
            // force a full repaint next frame.
            out.clear();
            backend.teardown(&mut out);
            full_redraw = true;
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(b"\x1b[2J");
            let _ = o.flush();
        }

        let now = now_ns();
        let mut elapsed = now - prev_t;
        prev_t = now;
        if elapsed > 8 * sim_dt_ns {
            elapsed = 8 * sim_dt_ns;
        }
        if elapsed > 0 {
            fps = fps * 0.9 + (NS_PER_SEC as f64 / elapsed as f64) * 0.1;
        }

        // Advance the sim only while playing; modals (help / quit prompt) freeze
        // it. Drop accumulated time when paused so resuming doesn't burst-step.
        if ui == Ui::Play {
            acc += elapsed;
            if input.jump_pressed() {
                pending_jump = true;
            }
            while acc >= sim_dt_ns {
                prev_pos = player.pos;
                player.step(
                    &arena.map,
                    sim_dt,
                    input.axis_x(),
                    pending_jump,
                    input.jump_held(),
                    input.down_held(),
                    &fp,
                );
                pending_jump = false; // consumed by the first substep only (no double-fire)
                acc -= sim_dt_ns;
                // Safety net (shouldn't happen in a closed box): respawn if it escapes.
                if player.pos.y > arena.map.px_h() + 64.0 {
                    player = Player::new(arena.map.spawn.0, arena.map.spawn.1);
                    prev_pos = player.pos;
                }
            }
        } else {
            acc = 0;
        }

        // Event-triggered effects (spawned at the player's feet in world px).
        let now = now_ns();
        let feet_x = player.pos.x + player.w / 2.0;
        let feet_y = player.pos.y + player.h;
        if player.did_double && !was_double {
            fx.spawn(&effects::PUFF, feet_x, feet_y, now); // double-jump burst
        }
        if player.grounded && !was_grounded {
            fx.spawn(&effects::DUST, feet_x, feet_y, now); // landing scuff
        }
        // Continuous friction sparks straddling the wall-contact line (his box
        // edge), so the burst is half on Munchii and half on the wall.
        if player.state == State::WallSliding && now.saturating_sub(last_spark_ns) >= NS_PER_SEC / 18 {
            let sx = feet_x + player.wall_dir as f64 * player.w / 2.0;
            let sy = player.pos.y + player.h * 0.6;
            fx.spawn(&effects::SPARK, sx, sy, now);
            last_spark_ns = now;
        }
        was_double = player.did_double;
        was_grounded = player.grounded;
        fx.update(now);

        // --- present (modal-aware) ---
        if ui == Ui::Help {
            render_help(&mut out, backend.name());
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.flush();
        } else {
            let alpha = acc as f64 / sim_dt_ns as f64;
            let rpos = prev_pos.lerp(player.pos, alpha);
            let disp_rows = arena.rows.saturating_sub(1);
            render_arena(&mut fb, &arena.map);

            // Pose by movement state (loops on wall-clock). During a wall-slide
            // Munchii faces AWAY from the wall (press left → on the left wall →
            // faces right), so the whole sprite mirrors.
            let anim = munchii::anim(pose_for(&player, input.down_held()));
            let n = anim.frames.len().max(1);
            let fi = (now / (NS_PER_SEC / anim.fps.max(1) as u64)) as usize % n;
            let face_left = if player.state == State::WallSliding {
                player.facing > 0
            } else {
                player.facing < 0
            };
            let lines: Vec<String> = if face_left {
                anim.frames[fi].iter().map(|l| flip_line(l)).collect()
            } else {
                anim.frames[fi].iter().map(|s| s.to_string()).collect()
            };
            let to_cells = |x: f64, y: f64| -> (i32, i32) {
                (
                    (x / arena.fb_w as f64 * arena.cols as f64).round() as i32,
                    (y / arena.fb_h as f64 * disp_rows as f64).round() as i32,
                )
            };

            // Munchii sprite aligned to his hitbox (feet on the box bottom,
            // centered across its width); same placement in every tier.
            let fw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as i32;
            let (box_left, box_top) = to_cells(rpos.x, rpos.y);
            let pcol = box_left + (munchii::W as i32 - fw) / 2;

            if backend.draws_overlay() {
                // Character tiers stamp Munchii + effects as overlays.
                let fx_render: Vec<(Vec<String>, (u8, u8, u8), i32, i32, i32)> = fx
                    .render(now)
                    .into_iter()
                    .map(|(frame, tint, z, x, y)| {
                        let fl: Vec<String> = frame.iter().map(|s| s.to_string()).collect();
                        let w = fl.iter().map(|l| l.chars().count()).max().unwrap_or(0) as i32;
                        let (cx, cy) = to_cells(x, y);
                        (fl, tint, z, cx - w / 2, cy)
                    })
                    .collect();

                let mut overlays: Vec<Overlay> = Vec::with_capacity(1 + fx_render.len());
                overlays.push(Overlay { lines: &lines, col: pcol, row: box_top, tint: None, z: 0 });
                for (fl, tint, z, col, row) in &fx_render {
                    overlays.push(Overlay { lines: fl, col: *col, row: *row, tint: Some(*tint), z: *z });
                }
                backend.present(&mut out, &fb, arena.cols, disp_rows, full_redraw, &overlays);
            } else {
                // Pixel tiers: rasterize Munchii + effects into the framebuffer,
                // z-ordered around him (behind first, then in front).
                let cpw = arena.fb_w as f64 / arena.cols.max(1) as f64;
                let cph = arena.fb_h as f64 / disp_rows.max(1) as f64;
                let fxr = fx.render(now);
                for &(frame, tint, z, x, y) in &fxr {
                    if z < 0 {
                        draw_effect_pixels(&mut fb, frame, tint, x, y, cpw, cph);
                    }
                }
                draw_sprite_pixels(&mut fb, &lines, pcol as f64 * cpw, box_top as f64 * cph, cpw, cph);
                for &(frame, tint, z, x, y) in &fxr {
                    if z >= 0 {
                        draw_effect_pixels(&mut fb, frame, tint, x, y, cpw, cph);
                    }
                }
                backend.present(&mut out, &fb, arena.cols, disp_rows, full_redraw, &[]);
            }
            full_redraw = false;
            if ui == Ui::ConfirmQuit {
                render_quit_prompt(&mut status, arena.rows, arena.cols);
            } else {
                render_status(&mut status, &player, score, fps, backend.name(), arena.rows, arena.cols);
            }
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(status.as_bytes());
            let _ = o.flush();
        }

        // Log the first frame's encoded size (the bandwidth tell) and a periodic
        // heartbeat so a hang/stall is visible in the log.
        frame += 1;
        if frame == 1 || frame % 120 == 0 {
            dlog!(
                "frame {frame}: backend={} encoded {} bytes, fps={fps:.0}, pos=({:.0},{:.0}) state={}",
                backend.name(), out.len(), player.pos.x, player.pos.y, state_letter(player.state)
            );
        }

        next += sim_dt_ns;
        let nn = now_ns();
        if next < nn {
            next = nn; // fell behind; don't spiral
        }
        sleep_until_ns(next, spin_margin);
    }
    drop(guard);
    eprintln!("scamp: bye.");
}

// ---------------------------------------------------------------------------
// Headless verification (scripted scenarios + PNG dumps + numeric asserts)
// ---------------------------------------------------------------------------

fn dump_png(dir: &str, name: &str, fb: &Framebuffer) {
    let path = format!("{dir}/{name}.png");
    scamper::png::write_file(&path, fb.width, fb.height, &fb.px).expect("write png");
    eprintln!("  wrote {path}");
}

fn run_verify(dir: &str) {
    let _ = std::fs::create_dir_all(dir);
    let fp = FeelParams::default();
    eprintln!("== scamp verify ==");

    // Scenario 1: sandbox traversal — run right, jump, double jump.
    {
        let map = build_sandbox();
        let mut p = Player::new(map.spawn.0, map.spawn.1);
        let mut fb = Framebuffer::new(map.px_w() as usize, map.px_h() as usize);
        let dt = 1.0 / 60.0;
        // settle
        for _ in 0..40 {
            p.step(&map, dt, 0.0, false, false, false, &fp);
        }
        render(&mut fb, &map, p.pos, &p);
        dump_png(dir, "01_spawn", &fb);
        // run right for ~50 frames
        let mut max_speed: f64 = 0.0;
        for _ in 0..50 {
            p.step(&map, dt, 1.0, false, false, false, &fp);
            max_speed = max_speed.max(p.vel.x.abs());
        }
        render(&mut fb, &map, p.pos, &p);
        dump_png(dir, "02_running", &fb);
        eprintln!("  run max |vx| = {max_speed:.1} px/s (cap {})", fp.max_run);
        // jump (single) and capture apex
        let y_before = p.pos.y;
        p.step(&map, dt, 1.0, true, true, false, &fp);
        let mut apex = p.pos.y;
        for _ in 0..18 {
            p.step(&map, dt, 1.0, false, true, false, &fp);
            apex = apex.min(p.pos.y);
        }
        // double jump mid-air
        p.step(&map, dt, 1.0, true, true, false, &fp);
        let air_after = p.air_jumps;
        for _ in 0..16 {
            p.step(&map, dt, 1.0, false, true, false, &fp);
            apex = apex.min(p.pos.y);
        }
        render(&mut fb, &map, p.pos, &p);
        dump_png(dir, "03_double_jump", &fb);
        eprintln!(
            "  jump rise = {:.1}px, air_jumps after double = {} (did_double={})",
            y_before - apex,
            air_after,
            p.did_double
        );
        assert!(y_before - apex > 30.0, "jump should gain meaningful height");
    }

    // Scenario 2: wall slide + wall jump on a dedicated wall map.
    {
        let map = wall_test_map();
        let mut p = Player::new(map.spawn.0, map.spawn.1);
        let mut fb = Framebuffer::new(map.px_w() as usize, map.px_h() as usize);
        let dt = 1.0 / 60.0;
        // Drift right into the wall while falling → should wall-slide.
        let mut slid = false;
        let mut min_fall_while_sliding = f64::INFINITY;
        let mut free_fall_peak: f64 = 0.0;
        for i in 0..60 {
            p.step(&map, dt, 1.0, false, false, false, &fp);
            if p.state == State::WallSliding {
                slid = true;
                min_fall_while_sliding = min_fall_while_sliding.min(p.vel.y);
            } else if p.wall_dir == 0 && p.vel.y > 0.0 {
                free_fall_peak = free_fall_peak.max(p.vel.y);
            }
            if i == 20 {
                render(&mut fb, &map, p.pos, &p);
                dump_png(dir, "04_wallslide", &fb);
            }
        }
        eprintln!(
            "  wall-sliding seen = {slid}, clamped fall = {:.1} px/s (cap {}), free-fall peak ~{:.0}",
            if min_fall_while_sliding.is_finite() { min_fall_while_sliding } else { 0.0 },
            fp.wall_slide_max_fall,
            free_fall_peak
        );
        assert!(slid, "player should wall-slide against the wall");
        assert!(
            min_fall_while_sliding <= fp.wall_slide_max_fall + 1.0,
            "wall slide should clamp fall speed"
        );

        // Now wall jump: press jump while sliding.
        let vx_before = p.vel.x;
        let wall_dir = p.wall_dir;
        p.step(&map, dt, 1.0, true, true, false, &fp);
        eprintln!(
            "  wall jump: wall_dir was {wall_dir}, vx {:.1} -> {:.1}, vy = {:.1}",
            vx_before, p.vel.x, p.vel.y
        );
        assert!(p.vel.y < -100.0, "wall jump should launch upward");
        // pushed away from wall (wall was on the right => vx should be negative)
        assert!(
            p.vel.x * (wall_dir as f64) < 0.0,
            "wall jump should push away from the wall"
        );
        for _ in 0..14 {
            p.step(&map, dt, 0.0, false, true, false, &fp);
        }
        render(&mut fb, &map, p.pos, &p);
        dump_png(dir, "05_walljump", &fb);
    }

    // Scenario 3: the box arena test app — verify a window-sized box is closed,
    // the player can run the floor, hits a wall, and wall-jumps, all in bounds.
    {
        // Synthetic 80x24 window reporting pixels (like Kitty would).
        let ws = terminal::WinSize { cols: 80, rows: 24, xpix: 800, ypix: 480 };
        let arena = build_arena(ws);
        let mut p = Player::new(arena.map.spawn.0, arena.map.spawn.1);
        let mut fb = Framebuffer::new(arena.fb_w, arena.fb_h);
        let dt = 1.0 / 60.0;
        let interior_max_x = arena.map.px_w() - TILE - p.w;

        // settle, then run right into the far wall.
        for _ in 0..30 {
            p.step(&arena.map, dt, 0.0, false, false, false, &fp);
        }
        let grounded_at_spawn = p.grounded;
        let mut hit_right_wall = false;
        let mut max_x: f64 = 0.0;
        for _ in 0..400 {
            p.step(&arena.map, dt, 1.0, false, false, false, &fp);
            max_x = max_x.max(p.pos.x);
            if p.wall_dir > 0 {
                hit_right_wall = true;
            }
            // INVARIANT: never escapes the box.
            assert!(
                p.pos.x >= TILE - 1.0 && p.pos.x <= interior_max_x + 1.0,
                "player left the arena horizontally: x={}",
                p.pos.x
            );
            assert!(p.pos.y >= 0.0 && p.pos.y <= arena.map.px_h(), "player left vertically: y={}", p.pos.y);
        }
        render(&mut fb, &arena.map, p.pos, &p);
        dump_png(dir, "06_arena_wall", &fb);
        eprintln!(
            "  arena {}x{} tiles, fb {}x{}px; grounded@spawn={grounded_at_spawn}, reached x={max_x:.0} (wall@{interior_max_x:.0}), hit_wall={hit_right_wall}",
            arena.map.w, arena.map.h, arena.fb_w, arena.fb_h
        );
        assert!(grounded_at_spawn, "player should spawn standing on the arena floor");
        assert!(hit_right_wall, "running right should reach the arena wall");
    }

    eprintln!("== all scenarios passed ==");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_is_a_closed_box_sized_to_window() {
        let ws = terminal::WinSize { cols: 80, rows: 24, xpix: 800, ypix: 480 };
        let a = build_arena(ws);
        // reserves one text row: usable height < full height
        assert!(a.fb_h <= 480 - (480 / 24));
        // border is solid all the way round; interior corner is open
        assert!(a.map.is_solid(0, 0));
        assert!(a.map.is_solid(a.map.w as i32 - 1, a.map.h as i32 - 1));
        assert!(a.map.is_solid(5, 0) && a.map.is_solid(5, a.map.h as i32 - 1));
        assert!(a.map.is_solid(0, 3) && a.map.is_solid(a.map.w as i32 - 1, 3));
        assert!(!a.map.is_solid(3, 3), "interior should be open");
    }

    #[test]
    fn arena_falls_back_without_pixel_size() {
        // Terminals that don't report pixels (xpix/ypix == 0) still get a sane box.
        let a = build_arena(terminal::WinSize { cols: 80, rows: 24, xpix: 0, ypix: 0 });
        assert!(a.map.w >= 6 && a.map.h >= 6);
        assert!(a.fb_w > 0 && a.fb_h > 0);
    }

    #[test]
    fn tiny_window_clamps_to_min_box() {
        let a = build_arena(terminal::WinSize { cols: 1, rows: 1, xpix: 16, ypix: 16 });
        assert!(a.map.w >= 6 && a.map.h >= 6, "must not produce a degenerate arena");
    }

    #[test]
    fn clamp_into_arena_rescues_embedded_player() {
        let a = build_arena(terminal::WinSize { cols: 40, rows: 12, xpix: 400, ypix: 240 });
        let mut p = Player::new(0.0, 0.0); // jammed into the top-left corner walls
        clamp_into_arena(&mut p, &a);
        assert!(p.pos.x >= TILE && p.pos.y >= TILE, "should be pushed into the interior");
        assert!(!a.map.overlaps(p.pos.x, p.pos.y, p.w, p.h), "clamped pos must be wall-free");
        assert_eq!(p.vel, Vec2::ZERO, "clamping should kill leftover velocity");
    }

    #[test]
    fn status_line_underlines_help_and_never_overflows() {
        let p = Player::new(10.0, 10.0);
        let mut s = String::new();
        // narrow terminal: must truncate well within the width, no wrap.
        render_status(&mut s, &p, 0, 60.0, "kitty", 24, 20);
        assert!(s.contains("\x1b[4mh\x1b[24m"), "h help affordance should be underlined");
        assert!(s.contains("\x1b[24;1H"), "should position to the last row");
        // strip escapes; visible text must fit in cols-1.
        let visible: String = strip_ansi(&s);
        assert!(visible.len() <= 19, "visible status {:?} exceeds width", visible);
    }

    #[test]
    fn help_screen_lists_controls_and_backends() {
        let mut out = Vec::new();
        render_help(&mut out, "text");
        let s = String::from_utf8(out).unwrap();
        for needle in ["SCAMPER", "Move", "Jump", "Wall", "Tab", "kitty", "text", "ascii", "mono", "quit"] {
            assert!(s.contains(needle), "help should mention {needle:?}");
        }
        // reflects the active backend
        assert!(s.contains("now: text"), "help should show the active backend");
    }

    #[test]
    fn quit_prompt_fits_and_offers_yes_no() {
        let mut s = String::new();
        render_quit_prompt(&mut s, 24, 40);
        let visible = strip_ansi(&s);
        assert!(visible.contains("Y") && visible.contains("N"), "should offer Y/N: {visible:?}");
        assert!(visible.len() <= 39, "quit prompt {visible:?} exceeds width");
        assert!(s.contains("\x1b[24;1H"), "should position to the last row");
    }

    // crude ANSI stripper for the width assertion
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // skip CSI: ESC [ ... <final letter>
                if chars.peek() == Some(&'[') {
                    chars.next();
                    while let Some(&d) = chars.peek() {
                        chars.next();
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

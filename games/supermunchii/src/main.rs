//! `scamp` — the game binary: a sandbox platformer level driven by keyboard,
//! rendered to a Kitty terminal. Also a headless `verify` mode that runs scripted
//! scenarios and dumps PNGs (for development on a box without a Kitty terminal).

use scamper::backend::{AsciiBackend, Backend, KittyBackend, MonoBackend, Overlay, TextBackend};
use scamper::capture::{self, InputFrame, Recording, Snapshots};
use scamper::munchii;
use scamper::framebuffer::{Framebuffer, Rgba};
use scamper::input::{Input, K_DOWN, K_ESC, K_HELP, K_N, K_Q, K_S, K_T, K_TAB, K_Y};
use scamper::level::art::{self, Theme};
use scamper::level::ir::Level;
use scamper::level::world::{camera, LevelWorld};
use scamper::math::Vec2;
use scamper::player::{FeelParams, Player, State};
use scamper::sim::{Sim, SIM_DT_NS};
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
    // Capture panics into the log before the terminal guard wraps the hook with
    // teardown — otherwise a crash behind the alt-screen leaves no trace.
    scamper::dbg::install_panic_logger();
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
        Some("record") => {
            // `record <name>`: live play, capturing per-tick inputs; the gated quit
            // (Q→Y) finalizes and writes the capture.
            match nth_nonflag(&args, 1) {
                Some(name) if capture::valid_name(name) => run_live(Some(name.to_string())),
                Some(name) => {
                    eprintln!("record: invalid capture name {name:?} (use letters, digits, . _ -)");
                    std::process::exit(2);
                }
                None => {
                    eprintln!("usage: scamp record <name>");
                    std::process::exit(2);
                }
            }
        }
        Some("replay") => {
            let mode = if args.iter().any(|a| a == "--bless") {
                ReplayMode::Bless
            } else if args.iter().any(|a| a == "--check") {
                ReplayMode::Check
            } else {
                ReplayMode::Play
            };
            match nth_nonflag(&args, 1) {
                Some(name) => run_replay(name, mode),
                None => {
                    eprintln!("usage: scamp replay <name> [--check|--bless]");
                    std::process::exit(2);
                }
            }
        }
        Some("captures") => run_captures(),
        Some("import") => run_import(&args),
        Some("level-info") => run_level_info(&args),
        Some("tiles") => run_tiles(),
        Some("play") => run_play(nth_nonflag(&args, 1).unwrap_or("levels/yard-romp-1.lvl")),
        Some("soak") => run_soak(nth_nonflag(&args, 1).unwrap_or("imported/lvl")),
        _ => run_live(None),
    }
}

/// `scamp import <in.tscn> <out.lvl> [--theme <t>] [--id <id>]` — offline dev tool:
/// convert a Godot scene to our Level IR. (CAMPAIGN_PLAN.md §4.)
fn run_import(args: &[String]) {
    let (input, output) = match (nth_nonflag(args, 1), nth_nonflag(args, 2)) {
        (Some(i), Some(o)) => (i, o),
        _ => {
            eprintln!("usage: scamp import <in.tscn> <out.lvl> [--theme <t>] [--id <id>]\n  (theme is auto-inferred from the scene if --theme is omitted; inheritance is resolved)");
            std::process::exit(2);
        }
    };
    // `--theme` is now an optional override; without it the importer infers the
    // theme from the scene (resolving inheritance) and falls back to a name guess.
    let theme_override = flag_value(args, "--theme");
    // Default id from the output filename stem.
    let default_id = std::path::Path::new(output).file_stem().and_then(|s| s.to_str()).unwrap_or("level");
    let id = flag_value(args, "--id").unwrap_or(default_id);

    let imp = match scamper::level::import_scene_file(std::path::Path::new(input), id, theme_override) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("import: {input}: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = std::fs::write(output, imp.level.to_text()) {
        eprintln!("import: cannot write {output}: {e}");
        std::process::exit(2);
    }
    let l = &imp.level;
    eprintln!(
        "imported {input} -> {output}: theme={}, {}x{} tiles, {} tile-spans, {} entities, spawn {:?}, goal {}",
        l.theme, l.w, l.h, l.tiles.len(), l.entities.len(), l.spawn,
        l.goal.as_ref().map(|g| format!("({},{})", g.x, g.y)).unwrap_or_else(|| "none".into()),
    );
    for w in &imp.warnings {
        eprintln!("  warning: {w}");
    }
}

/// `scamp level-info <file.lvl>` — print stats + an ascii map of a level.
fn run_level_info(args: &[String]) {
    let path = match nth_nonflag(args, 1) {
        Some(p) => p,
        None => {
            eprintln!("usage: scamp level-info <file.lvl>");
            std::process::exit(2);
        }
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("level-info: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let lvl = match scamper::level::Level::from_text(&text) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("level-info: {path}: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "{}  theme={}  {}x{} tiles  spawn={:?}  entities={}  checkpoints={}",
        lvl.id, lvl.theme, lvl.w, lvl.h, lvl.spawn, lvl.entities.len(), lvl.checkpoints.len()
    );
    if let Some(g) = &lvl.goal {
        println!("goal: {} at ({},{})", g.kind, g.x, g.y);
    }
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for e in &lvl.entities {
        *counts.entry(e.kind.as_str()).or_insert(0) += 1;
    }
    if !counts.is_empty() {
        let summary: Vec<String> = counts.iter().map(|(k, n)| format!("{k}×{n}")).collect();
        println!("entities: {}", summary.join(", "));
    }
    println!("\n{}", lvl.ascii_preview());
}

/// Value of a `--flag <value>` pair in `args`, if present.
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

/// The `n`-th non-flag argument after the program name (0 = the subcommand).
fn nth_nonflag(args: &[String], n: usize) -> Option<&str> {
    args.iter().skip(1).filter(|a| !a.starts_with("--")).nth(n).map(|s| s.as_str())
}

// ---------------------------------------------------------------------------
// Tile preview — see every tile kind across all four backends + every theme,
// so we can confirm they read distinctly from mono ASCII up to Kitty pixels.
// ---------------------------------------------------------------------------

/// `scamp tiles`: a 3×3 grid of tile-kind patches. Tab cycles the graphics
/// backend, `t` cycles the theme, q/Esc quits.
fn run_tiles() {
    let guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("tiles needs an interactive terminal ({e}).");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();
    let mut input = Input::new(kitty_kbd);
    let mut backend: Box<dyn Backend> = Box::new(KittyBackend::new());
    let switch_backend = make_switch_backend();

    // Layout: each kind is a `PATCH`×`PATCH` field of tiles, on a grid with a
    // one-tile gutter, so repeating tiles read as a surface.
    const PATCH: usize = 3;
    const GAP: usize = 1;
    const COLS: usize = 3;
    let rows = art::KINDS.len().div_ceil(COLS);
    let step = PATCH + GAP;
    let t = TILE as usize;
    let fb_w = (COLS * step + GAP) * t;
    let fb_h = (rows * step + GAP) * t;
    let mut fb = Framebuffer::new(fb_w, fb_h);

    let mut out: Vec<u8> = Vec::new();
    let mut status = String::new();
    let mut theme_i = 0usize;
    let mut dirty = true; // only re-encode when something changed (kitty is heavy)

    loop {
        if terminal::quit_requested() || input.quit {
            break;
        }
        input.poll();
        if input.quit || input.pressed(K_Q) || input.pressed(K_ESC) {
            break;
        }
        if input.pressed(K_TAB) {
            switch_backend(&mut backend);
            dirty = true;
        }
        if input.pressed(K_T) {
            theme_i = (theme_i + 1) % Theme::ALL.len();
            dirty = true;
        }
        if terminal::take_resize() {
            out.clear();
            backend.teardown(&mut out);
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(b"\x1b[2J");
            let _ = o.flush();
            dirty = true;
        }

        if dirty {
            let theme = Theme::ALL[theme_i];
            let pal = art::palette(theme);
            fb.clear(pal.sky);
            for (i, &kind) in art::KINDS.iter().enumerate() {
                let (gx, gy) = (i % COLS, i / COLS);
                let ox = ((GAP + gx * step) * t) as i32;
                let oy = ((GAP + gy * step) * t) as i32;
                for ty in 0..PATCH {
                    for tx in 0..PATCH {
                        art::draw_tile(&mut fb, ox + (tx * t) as i32, oy + (ty * t) as i32, kind, &pal);
                    }
                }
            }
            let ws = terminal::query_winsize();
            let cols = ws.cols.max(1);
            let play_rows = ws.rows.saturating_sub(1).max(1);
            backend.present(&mut out, &fb, cols, play_rows, true, &[]);
            render_tiles_status(&mut status, theme, backend.name(), ws.rows, ws.cols);
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(status.as_bytes());
            let _ = o.flush();
            dirty = false;
        }

        sleep_until_ns(now_ns() + NS_PER_SEC / 30, 1_000_000);
    }
    drop(guard);
    eprintln!("scamp: tiles done.");
}

/// Status row for the tile preview: theme, backend, the grid legend, and controls.
fn render_tiles_status(buf: &mut String, theme: Theme, backend: &str, rows: u16, cols: u16) {
    use std::fmt::Write;
    // Kinds in grid reading order (3 per row), so the legend maps to the patches.
    let names: Vec<&str> = art::KINDS.iter().map(|k| k.as_str()).collect();
    let grid = names.chunks(3).map(|r| r.join(" ")).collect::<Vec<_>>().join(" / ");
    let mut plain = String::new();
    let _ = write!(plain, "TILES  theme:{} (t)  ·  Tab gfx:{backend}  ·  q quit   [{grid}]", theme.name());
    let maxw = (cols as usize).saturating_sub(1);
    if plain.len() > maxw {
        plain.truncate(maxw);
    }
    buf.clear();
    let _ = write!(buf, "\x1b[{rows};1H\x1b[2K\x1b[7m{plain}\x1b[0m");
}

// ---------------------------------------------------------------------------
// Level runtime — play an imported/authored level: scrolling camera, tile
// collision, hazards, the goal (level end), and pipe warps (CAMPAIGN_PLAN.md §6).
// ---------------------------------------------------------------------------

/// Viewport geometry for the level camera: an internal framebuffer sized to a
/// whole number of tiles (so backends keep dimensional parity) plus the terminal
/// cell area it scales across. Mirrors `build_arena`, but it's a *window* onto a
/// larger level rather than the whole arena.
fn play_view(ws: terminal::WinSize) -> (usize, usize, u16, u16) {
    let tile = TILE as usize;
    let cpt_x = tile / CELL_PX; // cells per tile, horizontally (4)
    let cpt_y = tile / CELL_PH; // cells per tile, vertically (2)
    let max_tiles = MAX_INTERNAL_DIM / tile;
    let view_tw = (ws.cols.max(20) as usize / cpt_x).clamp(6, max_tiles);
    let view_th = ((ws.rows.max(6) as usize - 1) / cpt_y).clamp(5, max_tiles);
    (view_tw * tile, view_th * tile, (view_tw * cpt_x) as u16, (view_th * cpt_y) as u16)
}

fn load_level_file(path: &str) -> std::io::Result<Level> {
    Level::from_text(&std::fs::read_to_string(path)?)
}

/// The next `*.lvl` file (alphabetical) in the same directory as `current`, or
/// `None` if `current` is the last one. Used to auto-advance on level completion.
fn next_level_path(current: &str) -> Option<String> {
    let p = std::path::Path::new(current);
    let dir = p.parent()?;
    let curname = p.file_name()?;
    let mut sibs: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|q| q.extension().map(|x| x == "lvl").unwrap_or(false))
        .collect();
    sibs.sort();
    let idx = sibs.iter().position(|q| q.file_name() == Some(curname))?;
    sibs.get(idx + 1).map(|q| q.to_string_lossy().into_owned())
}

/// A fresh sim placing the player at `spawn` px (default ~1-tile hitbox).
fn sim_at(spawn: (f64, f64)) -> Sim {
    Sim::new(Player::new(spawn.0, spawn.1), spawn)
}

/// Parse a warp target `"<id>@tx,ty"` (empty id = same level) into the level to
/// load and the spawn pixel. `dir` is the directory of the current level file.
fn parse_warp(target: &str, dir: &std::path::Path, current: &Level) -> Option<(Level, (f64, f64))> {
    let (id, coords) = target.split_once('@')?;
    let (xs, ys) = coords.split_once(',')?;
    let (tx, ty): (i32, i32) = (xs.trim().parse().ok()?, ys.trim().parse().ok()?);
    let lvl = if id.is_empty() {
        current.clone()
    } else {
        load_level_file(dir.join(format!("{id}.lvl")).to_str()?).ok()?
    };
    Some((lvl, (tx as f64 * TILE, ty as f64 * TILE)))
}

fn run_play(path: &str) {
    let mut level = match load_level_file(path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("play: cannot load level {path}: {e}");
            std::process::exit(2);
        }
    };
    let dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let mut cur_path = path.to_string();

    let guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("play needs an interactive terminal ({e}).");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();
    let mut input = Input::new(kitty_kbd);
    let switch_backend = make_switch_backend();
    let mut backend: Box<dyn Backend> = Box::new(KittyBackend::new());

    let mut world = LevelWorld::from_level(&level);
    let mut sim = sim_at(world.spawn);

    let (mut fb_w, mut fb_h, mut cols, mut rows) = play_view(terminal::query_winsize());
    let mut fb = Framebuffer::new(fb_w, fb_h);
    let mut out: Vec<u8> = Vec::new();
    let mut status = String::new();
    let mut full_redraw = true;
    let mut pending_jump = false;
    let mut won = false;
    let mut won_at: u64 = 0; // ns timestamp when the level was completed

    let spin = 1_000_000u64;
    let mut acc: u64 = 0;
    let mut prev_t = now_ns();
    let mut next = now_ns();

    loop {
        if terminal::quit_requested() || input.quit {
            break;
        }
        input.poll();
        if input.quit || input.pressed(K_Q) || input.pressed(K_ESC) {
            break;
        }
        if input.pressed(K_TAB) {
            switch_backend(&mut backend);
            full_redraw = true;
        }
        if terminal::take_resize() {
            let v = play_view(terminal::query_winsize());
            fb_w = v.0;
            fb_h = v.1;
            cols = v.2;
            rows = v.3;
            fb.resize(fb_w, fb_h);
            out.clear();
            backend.teardown(&mut out);
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(b"\x1b[2J");
            let _ = o.flush();
            full_redraw = true;
        }

        // --- advance physics (fixed timestep) unless the level is finished ---
        let now = now_ns();
        let mut elapsed = now - prev_t;
        prev_t = now;
        if elapsed > 8 * SIM_DT_NS {
            elapsed = 8 * SIM_DT_NS;
        }
        if input.jump_pressed() {
            pending_jump = true;
        }
        if !won {
            acc += elapsed;
            while acc >= SIM_DT_NS {
                let inp = InputFrame {
                    axis_x: input.axis_x() as i8,
                    jump_pressed: pending_jump,
                    jump_held: input.jump_held(),
                    down_held: input.down_held(),
                };
                pending_jump = false;
                sim.step(&world.map, inp);
                acc -= SIM_DT_NS;
            }
        } else {
            acc = 0;
        }

        let (px, py, pw_, ph_) = (sim.player.pos.x, sim.player.pos.y, sim.player.w, sim.player.h);
        // hazard (lava/water) → respawn
        if world.hazard_overlap(px, py, pw_, ph_) {
            sim = sim_at(world.spawn);
            full_redraw = true;
        }
        // goal reached → level complete; after a short beat, auto-advance to the
        // next sibling level (a debugging aid: walk the whole set without quitting).
        if !won {
            if let Some((gx, _)) = world.goal {
                if px + pw_ / 2.0 >= gx {
                    won = true;
                    won_at = now;
                }
            }
        } else if now.saturating_sub(won_at) >= 700 * 1_000_000 {
            if let Some((next, lvl)) = next_level_path(&cur_path).and_then(|p| load_level_file(&p).ok().map(|l| (p, l))) {
                cur_path = next;
                level = lvl;
                world = LevelWorld::from_level(&level);
                sim = sim_at(world.spawn);
                won = false;
                full_redraw = true;
            }
            // No next level → stay on the completed screen (q to quit).
        }
        // pipe warp: press down while standing on a warp with a destination
        if !won && (input.pressed(K_S) || input.pressed(K_DOWN)) {
            if let Some(target) = world.warp_at(px, py, pw_, ph_).and_then(|w| w.target.clone()) {
                if let Some((lvl, spawn)) = parse_warp(&target, &dir, &level) {
                    level = lvl;
                    world = LevelWorld::from_level(&level);
                    sim = sim_at(spawn);
                    full_redraw = true;
                }
            }
        }

        // --- render the camera window (shared with the headless soak harness) ---
        draw_play_frame(&mut fb, backend.as_mut(), &mut out, &world, &sim, fb_w, fb_h, cols, rows, full_redraw, input.down_held());
        full_redraw = false;
        render_play_status(&mut status, &level, sim.player.state, backend.name(), won, rows + 1, cols);
        {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(status.as_bytes());
            let _ = o.flush();
        }

        next += SIM_DT_NS;
        let nn = now_ns();
        if next < nn {
            next = nn;
        }
        sleep_until_ns(next, spin);
    }
    drop(guard);
    eprintln!("scamp: play done.");
}

/// Draw one play frame — the camera window of tiles, the goal post, and Munchii —
/// and present it through `backend` into `out`. Shared by `run_play` and the
/// headless `soak` crash-hunt so both exercise the identical render path.
#[allow(clippy::too_many_arguments)]
fn draw_play_frame(
    fb: &mut Framebuffer,
    backend: &mut dyn Backend,
    out: &mut Vec<u8>,
    world: &LevelWorld,
    sim: &Sim,
    fb_w: usize,
    fb_h: usize,
    cols: u16,
    rows: u16,
    full_redraw: bool,
    down_held: bool,
) {
    let pal = art::palette(world.theme);
    let pcx = sim.player.pos.x + sim.player.w / 2.0;
    let pcy = sim.player.pos.y + sim.player.h / 2.0;
    let (cam_x, cam_y) = camera(pcx, pcy, fb_w as f64, fb_h as f64, world.px_w(), world.px_h());
    let cpw = fb_w as f64 / cols.max(1) as f64; // px per terminal cell (w)
    let cph = fb_h as f64 / rows.max(1) as f64; // px per terminal cell (h)
    // Snap the camera so tiles always land on the same sample sub-grid. Pixel
    // backends (Kitty) scroll per-pixel; cell-sampling backends must snap to a
    // whole cell or static tiles flicker as you move (see Backend::pixel_exact).
    let (cam_x, cam_y) = if backend.pixel_exact() {
        (cam_x.floor(), cam_y.floor())
    } else {
        ((cam_x / cpw).floor() * cpw, (cam_y / cph).floor() * cph)
    };

    fb.clear(pal.sky);
    let t = TILE as i32;
    let tx0 = (cam_x / TILE).floor() as i32;
    let tx1 = ((cam_x + fb_w as f64) / TILE).ceil() as i32;
    let ty0 = (cam_y / TILE).floor() as i32;
    let ty1 = ((cam_y + fb_h as f64) / TILE).ceil() as i32;
    for ty in ty0..ty1 {
        for tx in tx0..tx1 {
            if let Some(kind) = world.kind_at(tx, ty) {
                art::draw_tile(fb, tx * t - cam_x as i32, ty * t - cam_y as i32, kind, &pal);
            }
        }
    }
    // goal post
    if let Some((gx, gy)) = world.goal {
        let sx = (gx - cam_x) as i32;
        fb.fill_rect(sx, 0, 2, fb_h as i32, Rgba::rgb(235, 235, 245));
        fb.fill_rect(sx - 7, (gy - cam_y) as i32, 7, 5, Rgba::rgb(232, 84, 84));
    }
    // Munchii himself, centered on the hitbox with his feet on its bottom edge.
    let anim = munchii::anim(pose_for(&sim.player, down_held));
    let n = anim.frames.len().max(1);
    let fi = (sim.clock() / (NS_PER_SEC / anim.fps.max(1) as u64)) as usize % n;
    let face_left = if sim.player.state == State::WallSliding {
        sim.player.facing > 0
    } else {
        sim.player.facing < 0
    };
    let lines: Vec<String> = if face_left {
        anim.frames[fi].iter().map(|l| flip_line(l)).collect()
    } else {
        anim.frames[fi].iter().map(|s| s.to_string()).collect()
    };
    let fw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
    let (mw, mh) = (fw * cpw, lines.len() as f64 * cph); // sprite footprint, px
    let cx = (sim.player.pos.x - cam_x) + sim.player.w / 2.0;
    let bottom = (sim.player.pos.y - cam_y) + sim.player.h;
    let (lx, ly) = (cx - mw / 2.0, bottom - mh);

    if backend.draws_overlay() {
        // character tiers: stamp Munchii's glyphs (one per cell) over the tiles
        let col = (lx / cpw).round() as i32;
        let row = (ly / cph).round() as i32;
        let ov = [Overlay { lines: &lines, col, row, tint: None, z: 0 }];
        backend.present(out, fb, cols, rows, full_redraw, &ov);
    } else {
        // pixel tiers: rasterize him into the framebuffer in his beagle colors
        draw_sprite_pixels(fb, &lines, lx, ly, cpw, cph);
        backend.present(out, fb, cols, rows, full_redraw, &[]);
    }
}

/// `supermunchii soak [dir]` — headless crash-hunt: load every `*.lvl` under `dir`
/// (default `imported/lvl`) and run each through the sim + render pipeline for a
/// few hundred ticks (walking right, jumping), catching panics per level. No
/// terminal needed; panic details land in `scamp.log` (run with `--debug`).
fn run_soak(dir: &str) {
    let mut files = Vec::new();
    collect_lvls(std::path::Path::new(dir), &mut files);
    files.sort();
    if files.is_empty() {
        eprintln!("soak: no .lvl files under {dir}");
        std::process::exit(2);
    }
    let mut ok = 0usize;
    let mut fails: Vec<String> = Vec::new();
    for path in &files {
        let p = path.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| soak_level(&p, 800))) {
            Ok(Ok(())) => ok += 1,
            Ok(Err(e)) => fails.push(format!("FAIL  {path}: {e}")),
            Err(_) => fails.push(format!("PANIC {path}  (message + backtrace in scamp.log if --debug)")),
        }
    }
    eprintln!("soak: {ok}/{} ok", files.len());
    for f in &fails {
        eprintln!("  {f}");
    }
    if !fails.is_empty() {
        std::process::exit(1);
    }
}

fn collect_lvls(dir: &std::path::Path, out: &mut Vec<String>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_lvls(&p, out);
            } else if p.extension().map(|x| x == "lvl").unwrap_or(false) {
                out.push(p.to_string_lossy().into_owned());
            }
        }
    }
}

/// Run one level headlessly for `ticks` ticks through the same sim + render as
/// `run_play`, **holding right and repeatedly jumping** — the input that carries
/// Munchii through most of any level. Renders periodically through all four
/// backends so backend-specific draw crashes surface too. Returns `Err` on a
/// clean failure; a panic propagates (the soak/test catches it per level).
fn soak_level(path: &str, ticks: u64) -> Result<(), String> {
    let level = load_level_file(path).map_err(|e| format!("load: {e}"))?;
    let world = LevelWorld::from_level(&level);
    let mut sim = sim_at(world.spawn);
    let ws = terminal::WinSize { cols: 80, rows: 24, xpix: 640, ypix: 384 };
    let (fb_w, fb_h, cols, rows) = play_view(ws);
    let mut fb = Framebuffer::new(fb_w, fb_h);
    let mut backends: [Box<dyn Backend>; 4] =
        [Box::new(KittyBackend::new()), Box::new(TextBackend::new()), Box::new(AsciiBackend::new()), Box::new(MonoBackend::new())];
    let mut out: Vec<u8> = Vec::new();

    for tick in 0..ticks {
        // Hold right; tap jump on a ~18-tick cadence (held ~10) to clear gaps.
        let inp = InputFrame { axis_x: 1, jump_pressed: tick % 18 == 0, jump_held: tick % 18 < 10, down_held: false };
        sim.step(&world.map, inp);
        let (px, py, pw, ph) = (sim.player.pos.x, sim.player.pos.y, sim.player.w, sim.player.h);
        if world.hazard_overlap(px, py, pw, ph) {
            sim = sim_at(world.spawn);
        }
        if tick % 15 == 0 {
            for b in backends.iter_mut() {
                draw_play_frame(&mut fb, b.as_mut(), &mut out, &world, &sim, fb_w, fb_h, cols, rows, true, false);
            }
        }
    }
    Ok(())
}

fn render_play_status(buf: &mut String, level: &Level, st: State, backend: &str, won: bool, rows: u16, cols: u16) {
    use std::fmt::Write;
    let mut plain = String::new();
    if won {
        let _ = write!(plain, "★ LEVEL COMPLETE — {} ★   → next level…   gfx:{backend} · q quit", level.id);
    } else {
        let _ = write!(plain, "{}  [{}]  {}   A/D · jump · ↓ pipe · Tab gfx:{backend} · q quit", level.id, level.theme, state_letter(st));
    }
    let maxw = (cols as usize).saturating_sub(1);
    if plain.len() > maxw {
        plain.truncate(maxw);
    }
    buf.clear();
    let _ = write!(buf, "\x1b[{rows};1H\x1b[2K\x1b[7m{plain}\x1b[0m");
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
/// window — so per-frame bandwidth is bounded regardless of window size.
const MAX_INTERNAL_DIM: usize = 384;

/// Framebuffer pixels per terminal cell. These DIVIDE the tile size (TILE=16),
/// which is what makes rendering identical across backends: with the framebuffer
/// sized to `tiles * TILE` and the cell grid to `tiles * (TILE/CELL_*)`, a tile
/// spans a whole number of cells, so a wall lands on the same cell boundaries
/// whether kitty scales the image continuously or the cell tiers sample it.
/// (4 wide × 8 tall ≈ a terminal cell's 1:2 aspect, so the image isn't stretched.)
const CELL_PX: usize = 4;
const CELL_PH: usize = 8;

fn build_arena(ws: terminal::WinSize) -> Arena {
    let term_cols = ws.cols.max(20) as usize;
    let term_rows = ws.rows.max(6) as usize;
    let play_rows = term_rows - 1; // reserve the bottom row for the status line
    let tile = TILE as usize;
    let cpt_x = tile / CELL_PX; // cells per tile, horizontally (= 4)
    let cpt_y = tile / CELL_PH; // cells per tile, vertically   (= 2)
    let max_tiles = MAX_INTERNAL_DIM / tile;

    // Tiles fill as much of the terminal as the bandwidth cap allows; the cell
    // grid is then an exact multiple of the tile grid (no quantization mismatch).
    let tiles_w = (term_cols / cpt_x).clamp(3, max_tiles);
    let tiles_h = (play_rows / cpt_y).clamp(3, max_tiles);
    let cols_used = tiles_w * cpt_x;
    let rows_used = tiles_h * cpt_y;

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
        fb_w: tiles_w * tile,
        fb_h: tiles_h * tile,
        rows: (rows_used + 1) as u16, // playfield rows + the status row
        cols: cols_used as u16,
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

/// Framebuffer-px → terminal-cell mapping for the current arena (round to nearest).
fn to_cells(arena: &Arena, x: f64, y: f64) -> (i32, i32) {
    let disp_rows = arena.rows.saturating_sub(1);
    (
        (x / arena.fb_w as f64 * arena.cols as f64).round() as i32,
        (y / arena.fb_h as f64 * disp_rows as f64).round() as i32,
    )
}

/// Munchii's sprite lines + cell placement (col, row) for a render position, with
/// the animation frame selected off the **tick clock** (so it's identical live and
/// on replay). Shared by the live present and the snapshot renderer.
fn munchii_overlay(arena: &Arena, sim: &Sim, pos: Vec2, clock: u64) -> (Vec<String>, i32, i32) {
    let player = &sim.player;
    let anim = munchii::anim(pose_for(player, sim.last_input.down_held));
    let n = anim.frames.len().max(1);
    let fi = (clock / (NS_PER_SEC / anim.fps.max(1) as u64)) as usize % n;
    // During a wall-slide Munchii faces AWAY from the wall, so the sprite mirrors.
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
    let fw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as i32;
    let (box_left, box_top) = to_cells(arena, pos.x, pos.y);
    let pcol = box_left + (munchii::W as i32 - fw) / 2;
    (lines, pcol, box_top)
}

/// Live effects as character-tier overlays (owned lines, tint, z, and cell col/row),
/// for the character backends and the `mono_text` snapshot.
type FxOverlay = (Vec<String>, (u8, u8, u8), i32, i32, i32);
fn fx_overlays_cells(arena: &Arena, sim: &Sim, clock: u64) -> Vec<FxOverlay> {
    sim.fx
        .render(clock)
        .into_iter()
        .map(|(frame, tint, z, x, y)| {
            let fl: Vec<String> = frame.iter().map(|s| s.to_string()).collect();
            let w = fl.iter().map(|l| l.chars().count()).max().unwrap_or(0) as i32;
            let (cx, cy) = to_cells(arena, x, y);
            (fl, tint, z, cx - w / 2, cy)
        })
        .collect()
}

/// Render the full scene (arena + Munchii + effects) into `out` via `backend`. The
/// single present path for both live play and visual replay: character tiers stamp
/// overlays, pixel tiers rasterize into the framebuffer, z-ordered around Munchii.
fn present_scene(
    out: &mut Vec<u8>,
    fb: &mut Framebuffer,
    arena: &Arena,
    sim: &Sim,
    backend: &mut Box<dyn Backend>,
    full_redraw: bool,
    alpha: f64,
) {
    let clock = sim.clock();
    let rpos = sim.prev_pos.lerp(sim.player.pos, alpha);
    let disp_rows = arena.rows.saturating_sub(1);
    render_arena(fb, &arena.map);
    let (lines, pcol, prow) = munchii_overlay(arena, sim, rpos, clock);

    if backend.draws_overlay() {
        let fxr = fx_overlays_cells(arena, sim, clock);
        let mut overlays: Vec<Overlay> = Vec::with_capacity(1 + fxr.len());
        overlays.push(Overlay { lines: &lines, col: pcol, row: prow, tint: None, z: 0 });
        for (fl, tint, z, col, row) in &fxr {
            overlays.push(Overlay { lines: fl, col: *col, row: *row, tint: Some(*tint), z: *z });
        }
        backend.present(out, fb, arena.cols, disp_rows, full_redraw, &overlays);
    } else {
        let cpw = arena.fb_w as f64 / arena.cols.max(1) as f64;
        let cph = arena.fb_h as f64 / disp_rows.max(1) as f64;
        let fxr = sim.fx.render(clock);
        for &(frame, tint, z, x, y) in &fxr {
            if z < 0 {
                draw_effect_pixels(fb, frame, tint, x, y, cpw, cph);
            }
        }
        draw_sprite_pixels(fb, &lines, pcol as f64 * cpw, prow as f64 * cph, cpw, cph);
        for &(frame, tint, z, x, y) in &fxr {
            if z >= 0 {
                draw_effect_pixels(fb, frame, tint, x, y, cpw, cph);
            }
        }
        backend.present(out, fb, arena.cols, disp_rows, full_redraw, &[]);
    }
}

/// Render the current scene to plain text via `backend::mono_text` — the keyframe
/// snapshot used for deterministic regression testing. Tick-clock driven, so two
/// replays of the same capture produce byte-identical text.
fn snapshot_text(fb: &mut Framebuffer, arena: &Arena, sim: &Sim) -> String {
    let clock = sim.clock();
    render_arena(fb, &arena.map);
    let (lines, pcol, prow) = munchii_overlay(arena, sim, sim.player.pos, clock);
    let fxr = fx_overlays_cells(arena, sim, clock);
    let mut overlays: Vec<Overlay> = Vec::with_capacity(1 + fxr.len());
    overlays.push(Overlay { lines: &lines, col: pcol, row: prow, tint: None, z: 0 });
    for (fl, tint, z, col, row) in &fxr {
        overlays.push(Overlay { lines: fl, col: *col, row: *row, tint: Some(*tint), z: *z });
    }
    let cols = arena.cols as usize;
    let disp_rows = arena.rows.saturating_sub(1) as usize;
    // Canonical form: rows joined by '\n' with no trailing newline, matching the
    // snapshot file's line-oriented round-trip (`Snapshots::{to,from}_text`).
    scamper::backend::mono_text(fb, cols, disp_rows, &overlays)
        .trim_end_matches('\n')
        .to_string()
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
fn render_status(buf: &mut String, p: &Player, score: u32, fps: f64, backend: &str, rows: u16, cols: u16, recording: bool) {
    use std::fmt::Write;
    let mut plain = String::new();
    let rec = if recording { "REC  " } else { "" };
    let _ = write!(
        plain,
        "{rec}h Help  |  Tab gfx:{backend}  |  Score {score}  |  {}  vx {:>4.0} vy {:>4.0}  |  {fps:>3.0} fps",
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

fn run_live(record_name: Option<String>) {
    let guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("scamp needs an interactive terminal (Kitty/Ghostty/foot). ({e})");
            eprintln!("Try: run it directly in a Kitty terminal, or `scamp verify <dir>` for a headless render dump.");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();

    let ws0 = terminal::query_winsize();
    let mut arena = build_arena(ws0);
    let mut fb = Framebuffer::new(arena.fb_w, arena.fb_h);
    let mut player = Player::new(arena.map.spawn.0, arena.map.spawn.1);
    fit_player_to_munchii(&mut player, &arena);
    let mut sim = Sim::new(player, arena.map.spawn);
    let mut input = Input::new(kitty_kbd);

    // Recording: capture the originating window + every per-tick InputFrame. While
    // recording the arena is frozen (resizes ignored) so the capture has a single
    // geometry — the load-bearing requirement for faithful replay.
    let mut recorder: Option<Recording> = record_name.as_ref().map(|n| Recording::new(n.clone(), ws0));
    dlog!(
        "live: kitty_kbd={kitty_kbd} winsize={ws0:?} record={:?} -> arena {}x{} tiles, internal image {}x{}px scaled across {}x{} cells, spawn=({:.0},{:.0})",
        record_name, arena.map.w, arena.map.h, arena.fb_w, arena.fb_h, arena.cols, arena.rows.saturating_sub(1), arena.map.spawn.0, arena.map.spawn.1
    );

    let mut out: Vec<u8> = Vec::new();
    let mut status = String::new();
    let score: u32 = 0;
    let mut fps = 60.0_f64;
    let mut backend: Box<dyn Backend> = Box::new(KittyBackend::new());
    let mut full_redraw = true; // force a complete repaint after switch/resize

    let spin_margin = 1_000_000u64; // 1ms
    let mut acc: u64 = 0;
    let mut prev_t = now_ns();
    let mut next = now_ns();
    let mut pending_jump = false; // latch a press until a sim tick consumes it
    let mut frame: u64 = 0;

    let mut ui = Ui::Play;
    let switch_backend = make_switch_backend();

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
        // Skipped while recording (the capture keeps one fixed geometry).
        if terminal::take_resize() && recorder.is_none() {
            let ws = terminal::query_winsize();
            arena = build_arena(ws);
            fb.resize(arena.fb_w, arena.fb_h);
            dlog!("resize: winsize={ws:?} -> arena {}x{} tiles, image {}x{}px", arena.map.w, arena.map.h, arena.fb_w, arena.fb_h);
            // Footprint depends on the window, so refit Munchii's hitbox to the
            // new arena and reseat him inside it (also rescues a shrink that would
            // otherwise trap him in a wall).
            fit_player_to_munchii(&mut sim.player, &arena);
            sim.prev_pos = sim.player.pos;
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
        if elapsed > 8 * SIM_DT_NS {
            elapsed = 8 * SIM_DT_NS;
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
            while acc >= SIM_DT_NS {
                // One tick = one InputFrame = one Player::step. jump_pressed is
                // consumed by the first tick of the frame only (no double-fire).
                let inp = InputFrame {
                    axis_x: input.axis_x() as i8,
                    jump_pressed: pending_jump,
                    jump_held: input.jump_held(),
                    down_held: input.down_held(),
                };
                pending_jump = false;
                if let Some(rec) = recorder.as_mut() {
                    rec.frames.push(inp);
                }
                sim.step(&arena.map, inp);
                acc -= SIM_DT_NS;
            }
        } else {
            acc = 0;
        }

        // --- present (modal-aware) ---
        if ui == Ui::Help {
            render_help(&mut out, backend.name());
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.flush();
        } else {
            let alpha = acc as f64 / SIM_DT_NS as f64;
            present_scene(&mut out, &mut fb, &arena, &sim, &mut backend, full_redraw, alpha);
            full_redraw = false;
            if ui == Ui::ConfirmQuit {
                render_quit_prompt(&mut status, arena.rows, arena.cols);
            } else {
                render_status(&mut status, &sim.player, score, fps, backend.name(), arena.rows, arena.cols, recorder.is_some());
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
                "frame {frame}: backend={} encoded {} bytes, fps={fps:.0}, tick={} pos=({:.0},{:.0}) state={}",
                backend.name(), out.len(), sim.tick, sim.player.pos.x, sim.player.pos.y, state_letter(sim.player.state)
            );
        }

        next += SIM_DT_NS;
        let nn = now_ns();
        if next < nn {
            next = nn; // fell behind; don't spiral
        }
        sleep_until_ns(next, spin_margin);
    }

    // Persist the recording on every exit path (gated quit, Ctrl-C, signal) so a
    // run is never lost. Restore the terminal first so the message is readable.
    if let Some(rec) = recorder {
        drop(guard);
        let dir = capture::captures_dir();
        match capture::save_recording(&dir, &rec) {
            Ok(p) => eprintln!("scamp: recorded {} ticks -> {}", rec.frames.len(), p.display()),
            Err(e) => eprintln!("scamp: FAILED to save recording {:?}: {e}", rec.name),
        }
        return;
    }
    drop(guard);
    eprintln!("scamp: bye.");
}

/// The active-backend cycle (kitty → text → ascii → mono → kitty), clearing the
/// old backend's output. Shared by live play and visual replay.
fn make_switch_backend() -> impl Fn(&mut Box<dyn Backend>) {
    |backend: &mut Box<dyn Backend>| {
        let mut o2: Vec<u8> = Vec::new();
        backend.teardown(&mut o2);
        {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&o2);
            let _ = o.write_all(b"\x1b[2J");
            let _ = o.flush();
        }
        *backend = match backend.name() {
            "kitty" => Box::new(TextBackend::new()) as Box<dyn Backend>,
            "text" => Box::new(AsciiBackend::new()) as Box<dyn Backend>,
            "ascii" => Box::new(MonoBackend::new()) as Box<dyn Backend>,
            _ => Box::new(KittyBackend::new()) as Box<dyn Backend>,
        };
        dlog!("backend -> {}", backend.name());
    }
}

// ---------------------------------------------------------------------------
// Replay — re-run a capture's per-tick inputs through the same tick-driven sim,
// either visually (a live window) or headless for snapshot regression testing.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum ReplayMode {
    Play,  // visual playback in a terminal
    Check, // headless: snapshot keyframes, diff against golden (CI)
    Bless, // headless: snapshot keyframes, write/update golden
}

/// Keyframe cadence for snapshot regression: every N ticks, plus the final tick.
const SNAPSHOT_INTERVAL: u64 = 30;

fn run_replay(name: &str, mode: ReplayMode) {
    let dir = capture::captures_dir();
    let rec = match capture::load_recording(&dir, name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("replay: cannot load capture {name:?}: {e}");
            eprintln!("  (looked in {})", dir.display());
            std::process::exit(2);
        }
    };
    let arena = build_arena(rec.win);
    let mut fb = Framebuffer::new(arena.fb_w, arena.fb_h);
    let mut player = Player::new(arena.map.spawn.0, arena.map.spawn.1);
    fit_player_to_munchii(&mut player, &arena);
    let mut sim = Sim::new(player, arena.map.spawn);

    match mode {
        ReplayMode::Play => replay_visual(&rec, &arena, &mut fb, sim),
        ReplayMode::Check | ReplayMode::Bless => {
            // Headless: replay every recorded tick, snapshotting at the keyframe
            // cadence. Pure + deterministic, so it doubles as the CI invariant.
            let keys = compute_keyframes(&rec, &arena, &mut fb, &mut sim);

            if mode == ReplayMode::Bless {
                let mut snaps = Snapshots::new(name);
                snaps.keys = keys;
                match capture::save_snapshots(&dir, &snaps) {
                    Ok(p) => eprintln!("replay {name}: blessed {} keyframes -> {}", snaps.keys.len(), p.display()),
                    Err(e) => {
                        eprintln!("replay {name}: FAILED to write snapshots: {e}");
                        std::process::exit(2);
                    }
                }
            } else {
                let golden = match capture::load_snapshots(&dir, name) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("replay {name}: no golden snapshots ({e}). Bless first: scamp replay {name} --bless");
                        std::process::exit(2);
                    }
                };
                let diffs = golden.diff(&keys);
                if diffs.is_empty() {
                    eprintln!("replay {name}: {} keyframes match golden ✓", keys.len());
                } else {
                    for d in &diffs {
                        eprintln!("{d}");
                    }
                    eprintln!("replay {name}: MISMATCH — replay diverged from golden");
                    std::process::exit(1);
                }
            }
        }
    }
}

/// Replay `rec` through `sim` headless, returning `mono_text` keyframes at the
/// snapshot cadence (always including tick 0 and the final tick). Deterministic:
/// the same capture + arena always yields byte-identical output.
fn compute_keyframes(rec: &Recording, arena: &Arena, fb: &mut Framebuffer, sim: &mut Sim) -> Vec<(u64, String)> {
    let mut keys: Vec<(u64, String)> = Vec::new();
    for inp in &rec.frames {
        if sim.tick % SNAPSHOT_INTERVAL == 0 {
            keys.push((sim.tick, snapshot_text(fb, arena, sim)));
        }
        sim.step(&arena.map, *inp);
    }
    if keys.last().map(|(t, _)| *t) != Some(sim.tick) {
        keys.push((sim.tick, snapshot_text(fb, arena, sim)));
    }
    keys
}

/// Visual replay: feed the capture's inputs back one tick per rendered frame at
/// 60 fps (reproducing the original 60 Hz pacing). Tab cycles backends, q quits.
fn replay_visual(rec: &Recording, arena: &Arena, fb: &mut Framebuffer, mut sim: Sim) {
    let guard = match terminal::TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("replay needs an interactive terminal ({e}). Use --check for headless.");
            return;
        }
    };
    let kitty_kbd = terminal::probe_kitty_keyboard();
    let mut input = Input::new(kitty_kbd);
    let mut out: Vec<u8> = Vec::new();
    let mut status = String::new();
    let mut backend: Box<dyn Backend> = Box::new(KittyBackend::new());
    let mut full_redraw = true;
    let switch_backend = make_switch_backend();
    let total = rec.frames.len();

    let spin_margin = 1_000_000u64;
    let mut next = now_ns();
    let mut i = 0usize;
    loop {
        if terminal::quit_requested() || input.quit {
            break;
        }
        input.poll();
        if input.quit || input.pressed(K_Q) || input.pressed(K_ESC) {
            break;
        }
        if input.pressed(K_TAB) {
            switch_backend(&mut backend);
            full_redraw = true;
        }
        if terminal::take_resize() {
            // Geometry is fixed to the capture; just repaint cleanly.
            out.clear();
            backend.teardown(&mut out);
            full_redraw = true;
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(b"\x1b[2J");
            let _ = o.flush();
        }

        // One recorded tick per frame; hold on the final frame when done.
        let done = i >= total;
        if !done {
            sim.step(&arena.map, rec.frames[i]);
            i += 1;
        }

        present_scene(&mut out, fb, arena, &sim, &mut backend, full_redraw, 0.0);
        full_redraw = false;
        render_replay_status(&mut status, &rec.name, sim.tick, total as u64, backend.name(), arena.rows, arena.cols, done);
        {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(status.as_bytes());
            let _ = o.flush();
        }

        next += SIM_DT_NS;
        let nn = now_ns();
        if next < nn {
            next = nn;
        }
        sleep_until_ns(next, spin_margin);
    }
    drop(guard);
    eprintln!("scamp: replay done.");
}

/// Replay's bottom status row: progress through the capture + backend + quit hint.
fn render_replay_status(buf: &mut String, name: &str, tick: u64, total: u64, backend: &str, rows: u16, cols: u16, done: bool) {
    use std::fmt::Write;
    let tail = if done { "done — q to quit" } else { "q quit" };
    let mut plain = String::new();
    let _ = write!(plain, "REPLAY {name}  |  tick {tick}/{total}  |  Tab gfx:{backend}  |  {tail}");
    let maxw = (cols as usize).saturating_sub(1);
    if plain.len() > maxw {
        plain.truncate(maxw);
    }
    buf.clear();
    let _ = write!(buf, "\x1b[{rows};1H\x1b[2K\x1b[7m{plain}\x1b[0m");
}

/// `scamp captures`: list recorded captures (and whether each has golden snapshots).
fn run_captures() {
    let dir = capture::captures_dir();
    let names = capture::list_captures(&dir);
    if names.is_empty() {
        eprintln!("no captures in {}", dir.display());
        eprintln!("record one with:  scamp record <name>");
        return;
    }
    println!("captures in {}:", dir.display());
    for n in names {
        let has_golden = capture::snapshot_path(&dir, &n).exists();
        let ticks = capture::load_recording(&dir, &n).map(|r| r.frames.len()).unwrap_or(0);
        println!("  {n}  ({ticks} ticks){}", if has_golden { "  [golden]" } else { "" });
    }
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

    /// Soak each given level by holding right and jumping; collect any that panic
    /// or error so the assertion names the offenders rather than dying on the first.
    fn soak_all(files: &[String], ticks: u64) -> Vec<String> {
        let mut fails = Vec::new();
        for path in files {
            let p = path.clone();
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| soak_level(&p, ticks))) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => fails.push(format!("{path}: {e}")),
                Err(_) => fails.push(format!("{path}: panicked (run `supermunchii soak --debug` for the backtrace)")),
            }
        }
        fails
    }

    /// The committed authored levels must survive a walk-right-and-jump playthrough
    /// in every backend. Runs in the normal suite (these levels are always present).
    #[test]
    fn authored_levels_survive_a_walkthrough() {
        let mut files = Vec::new();
        collect_lvls(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("levels"), &mut files);
        assert!(!files.is_empty(), "no authored levels found to soak");
        let fails = soak_all(&files, 1500);
        assert!(fails.is_empty(), "authored levels crashed:\n  {}", fails.join("\n  "));
    }

    /// Full sweep of the locally-imported level set (gitignored, so absent in CI).
    /// On-demand because it's hundreds of levels: `cargo test imported_levels -- --ignored`.
    #[test]
    #[ignore]
    fn imported_levels_survive_a_walkthrough() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../imported/lvl");
        if !root.is_dir() {
            eprintln!("no imported/lvl — skipping (run `supermunchii import` first)");
            return;
        }
        let mut files = Vec::new();
        collect_lvls(&root, &mut files);
        let fails = soak_all(&files, 800);
        assert!(fails.is_empty(), "{} imported levels crashed:\n  {}", fails.len(), fails.join("\n  "));
    }

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
        assert!(a.map.w >= 3 && a.map.h >= 3, "must not produce a degenerate arena");
    }

    #[test]
    fn backend_dimensional_parity() {
        // The framebuffer and cell grid must align so a tile spans a WHOLE number
        // of cells — that's what makes walls (and any tile-based scenery) the same
        // size whether kitty scales the image or the cell tiers sample it.
        for ws in [
            terminal::WinSize { cols: 80, rows: 24, xpix: 800, ypix: 480 },
            terminal::WinSize { cols: 77, rows: 45, xpix: 1386, ypix: 1620 },
            terminal::WinSize { cols: 200, rows: 60, xpix: 0, ypix: 0 },
            terminal::WinSize { cols: 20, rows: 6, xpix: 0, ypix: 0 },
        ] {
            let a = build_arena(ws);
            let cols = a.cols as usize;
            let disp_rows = a.rows.saturating_sub(1) as usize;
            assert!(cols > 0 && disp_rows > 0);
            assert_eq!(a.fb_w % cols, 0, "fb width must be a whole number of cells");
            assert_eq!(a.fb_h % disp_rows, 0, "fb height must be a whole number of cells");
            let cell_px = a.fb_w / cols;
            let cell_ph = a.fb_h / disp_rows;
            assert_eq!(TILE as usize % cell_px, 0, "a tile must span whole cells (horizontal)");
            assert_eq!(TILE as usize % cell_ph, 0, "a tile must span whole cells (vertical)");
            // and the framebuffer is an exact tile grid (no partial tiles)
            assert_eq!(a.fb_w % TILE as usize, 0);
            assert_eq!(a.fb_h % TILE as usize, 0);
        }
    }

    #[test]
    fn cell_blocks_tile_without_gaps() {
        // Sprite/effect rasterization: N glyph blocks must paint a contiguous span
        // of exactly floor(N*cpw) px (no gaps, no overlap, no ceil inflation), so a
        // sprite is the same size in the pixel tiers as its cell count in the
        // character tiers.
        let cpw = 3.7_f64;
        let cph = 5.0_f64;
        let n = 7usize;
        let mut fb = Framebuffer::new(64, 8);
        fb.clear(Rgba::rgb(0, 0, 0));
        for gc in 0..n {
            cell_block(&mut fb, 0.0, 0.0, gc, 0, cpw, cph, Rgba::rgb(255, 0, 0));
        }
        let expected = (n as f64 * cpw).floor() as usize;
        for x in 0..expected {
            assert_eq!(fb.px[x * 4], 255, "gap at column {x}");
        }
        assert_eq!(fb.px[expected * 4], 0, "painted past the cell span at {expected}");
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
        render_status(&mut s, &p, 0, 60.0, "kitty", 24, 20, false);
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

    // ---- record / replay (RECORD_REPLAY.md) ----

    /// A deterministic scripted run used both to bless the committed fixture and
    /// to drive the determinism tests: settle, run right into the wall, jump,
    /// double-jump, fast-fall — enough to exercise movement + effects.
    fn fixture_recording() -> Recording {
        let mut r = Recording::new("ci-smoke", terminal::WinSize { cols: 80, rows: 24, xpix: 800, ypix: 480 });
        let push = |r: &mut Recording, n: usize, f: InputFrame| {
            for _ in 0..n {
                r.frames.push(f);
            }
        };
        let z = InputFrame::default();
        push(&mut r, 20, z); // settle on the floor
        push(&mut r, 60, InputFrame { axis_x: 1, ..z }); // run right
        // jump (press one tick, hold the rise), still drifting right
        push(&mut r, 1, InputFrame { axis_x: 1, jump_pressed: true, jump_held: true, ..z });
        push(&mut r, 14, InputFrame { axis_x: 1, jump_held: true, ..z });
        // double jump
        push(&mut r, 1, InputFrame { axis_x: 1, jump_pressed: true, jump_held: true, ..z });
        push(&mut r, 12, InputFrame { axis_x: 1, jump_held: true, ..z });
        push(&mut r, 20, InputFrame { axis_x: 1, down_held: true, ..z }); // fast-fall
        push(&mut r, 30, z); // settle again
        r
    }

    fn arena_for(rec: &Recording) -> (Arena, Framebuffer, Sim) {
        let arena = build_arena(rec.win);
        let fb = Framebuffer::new(arena.fb_w, arena.fb_h);
        let mut player = Player::new(arena.map.spawn.0, arena.map.spawn.1);
        fit_player_to_munchii(&mut player, &arena);
        let sim = Sim::new(player, arena.map.spawn);
        (arena, fb, sim)
    }

    #[test]
    fn replay_keyframes_are_deterministic() {
        let rec = fixture_recording();
        let run = || {
            let (arena, mut fb, mut sim) = arena_for(&rec);
            compute_keyframes(&rec, &arena, &mut fb, &mut sim)
        };
        let a = run();
        let b = run();
        assert!(!a.is_empty(), "should produce keyframes");
        assert_eq!(a, b, "replaying the same capture must be byte-identical");
        // sanity: tick 0 is captured and the final tick is the frame count
        assert_eq!(a.first().unwrap().0, 0);
        assert_eq!(a.last().unwrap().0, rec.frames.len() as u64);
    }

    fn fixtures_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join("captures")
    }

    /// CI invariant: the committed capture, replayed headless, must reproduce its
    /// committed golden keyframes exactly. A physics or rasterizer change that
    /// alters behavior fails here. Regenerate intentionally with `bless_fixtures`.
    #[test]
    fn committed_fixture_matches_golden() {
        let dir = fixtures_dir();
        let rec = capture::load_recording(&dir, "ci-smoke").expect("load fixture capture");
        let golden = capture::load_snapshots(&dir, "ci-smoke").expect("load golden snapshots");
        let (arena, mut fb, mut sim) = arena_for(&rec);
        let keys = compute_keyframes(&rec, &arena, &mut fb, &mut sim);
        let diffs = golden.diff(&keys);
        assert!(diffs.is_empty(), "replay diverged from golden:\n{}", diffs.join("\n"));
    }

    /// Regenerate the committed fixture (capture + golden snapshots). Not run in
    /// the normal suite — invoke deliberately after an intended behavior change:
    ///   cargo test bless_fixtures -- --ignored
    #[test]
    #[ignore]
    fn bless_fixtures() {
        let dir = fixtures_dir();
        let rec = fixture_recording();
        capture::save_recording(&dir, &rec).expect("write fixture capture");
        let (arena, mut fb, mut sim) = arena_for(&rec);
        let keys = compute_keyframes(&rec, &arena, &mut fb, &mut sim);
        let mut snaps = Snapshots::new(&rec.name);
        snaps.keys = keys;
        let p = capture::save_snapshots(&dir, &snaps).expect("write golden snapshots");
        eprintln!("blessed fixture -> {} ({} keyframes)", p.display(), snaps.keys.len());
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

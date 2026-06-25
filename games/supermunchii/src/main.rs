//! `scamp` — the game binary: a sandbox platformer level driven by keyboard,
//! rendered to a Kitty terminal. Also a headless `verify` mode that runs scripted
//! scenarios and dumps PNGs (for development on a box without a Kitty terminal).

use scamper::backend::{AsciiBackend, Backend, KittyBackend, MonoBackend, Overlay, TextBackend};
use scamper::capture::{self, InputFrame, Recording, Snapshots};
use scamper::munchii;
use scamper::framebuffer::{Framebuffer, Rgba};
use scamper::input::{Input, K_DOWN, K_ESC, K_G, K_HELP, K_N, K_P, K_Q, K_S, K_T, K_TAB, K_X, K_Y};
use scamper::level::art::{self, Theme};
use scamper::level::ir::Level;
use scamper::level::world::{Bonk, LevelWorld};
use scamper::math::Vec2;
use scamper::mob::{aabb_overlap, stomp, Gait, Mob};
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
        Some("play") => match nth_nonflag(&args, 1) {
            Some(path) => run_play(path),
            None => run_play(&default_test_level()), // no level given → a fresh random stitch
        },
        Some("soak") => run_soak(nth_nonflag(&args, 1).unwrap_or("imported/lvl")),
        Some("mega") => run_mega(&args),
        Some("slice") => run_slice(&args),
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
    if plain.chars().count() > maxw {
        plain = plain.chars().take(maxw).collect(); // clamp by chars: multibyte-safe
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
/// Where per-level best clear-times persist (per-user home; falls back to cwd).
fn bests_path() -> std::path::PathBuf {
    let dir = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("."));
    dir.join(".supermunchii_bests")
}

/// Load the `id<TAB>seconds` best-time table (a missing/garbled file → empty).
fn load_bests() -> std::collections::HashMap<String, u32> {
    let mut m = std::collections::HashMap::new();
    if let Ok(text) = std::fs::read_to_string(bests_path()) {
        for line in text.lines() {
            if let Some((id, secs)) = line.split_once('\t') {
                if let Ok(s) = secs.trim().parse::<u32>() {
                    m.insert(id.to_string(), s);
                }
            }
        }
    }
    m
}

/// Persist the best-time table (best-effort; write errors are ignored).
fn save_bests(m: &std::collections::HashMap<String, u32>) {
    let mut out = String::new();
    for (id, secs) in m {
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{id}\t{secs}\n"));
    }
    let _ = std::fs::write(bests_path(), out);
}

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
    let mut respawn = world.spawn; // advances to the last checkpoint passed
    let mut next_cp = 0usize; // index of the next un-passed checkpoint
    let mut actors = build_actors(&world);
    let mut projectiles: Vec<Mob> = Vec::new(); // Munchii's Sudsballs
    let mut hostiles: Vec<Mob> = Vec::new(); // enemy thrown sticks
    let mut fire_cd: u32 = 0; // throw cooldown (frames)
    let mut aura_t: u64 = 0; // last bubble-aura emit (ns)
    let mut ambient_t: u64 = 0; // last ambient-particle emit (ns)
    let mut wallspark_t: u64 = 0; // last wall-slide friction spark (ns)
    let mut dash_t: u64 = 0; // last zoomies dash-trail emit (ns)
    let mut shake = scamper::shake::Shake::new(); // camera juice on impacts
    let mut frame_ix: u64 = 0; // render-frame counter (drives the shake tremble)
    let mut look_ahead: f64 = 0.0; // smoothed camera lead in the travel direction
    let mut zoomies: u32 = 0; // zoomies-treat speed burst, ticks remaining
    const ZOOMIES_TICKS: u32 = 360; // ~6s at the sim rate
    let mut glide = false; // Flutter Collar: hold jump while falling to glide
    let mut glide_t: u64 = 0; // last glide-feather emit (ns)
    let mut dash_cd: u32 = 0; // dash cooldown (ticks)
    let mut dashing: u32 = 0; // active dash window (ticks)
    const DASH_SPEED: f64 = 405.0;
    const DASH_FRAMES: u32 = 9;
    const DASH_CD: u32 = 40;
    let mut combo: u32 = 0; // consecutive air-pounces (resets on landing)
    let mut boss_hp: i32 = 3; // Baron Whiskers' health (only matters if he's present)
    let mut boss_cd: u32 = 0; // boss i-frames between pounces (ticks)
    let mut boss_t: u64 = 0; // last boss telegraph emit (ns)
    const GLIDE_FALL: f64 = 72.0; // capped descent (px/s) while gliding
    let base_max_run = sim.fp.max_run;
    let base_run_accel = sim.fp.run_accel;
    let mut kibble: u32 = 0;
    let mut next_1up: u32 = 100; // every 100 kibble → an extra life
    let mut lives: i32 = 3;
    let mut power = Power::Small;
    let mut invuln: u32 = 0; // ticks of post-hit invulnerability
    let mut invincible: u32 = 0; // Star Bone: enemy-immunity + bulldoze, ticks left
    let mut star_t: u64 = 0; // last star-sparkle emit (ns)
    const STAR_TICKS: u32 = 480; // ~8s of invincibility
    let mut skid_cd: u32 = 0; // throttle for skid-dust effects
    let mut help = false; // controls overlay (h)
    let mut paused = false; // freeze on 'p'
    let mut assist = false; // practice mode: invulnerable (toggle 'g')
    play_bgm(&level);

    let (mut fb_w, mut fb_h, mut cols, mut rows) = play_view(terminal::query_winsize());
    let mut fb = Framebuffer::new(fb_w, fb_h);
    let mut out: Vec<u8> = Vec::new();
    let mut status = String::new();
    let mut full_redraw = true;
    let mut pending_jump = false;
    let mut won = false;
    let mut won_at: u64 = 0; // ns timestamp when the level was completed
    let mut celebrated = false; // win-flourish fired (edge-triggered off `won`)
    let mut bests = load_bests(); // persisted best clear-time per level id
    let mut is_new_best = false; // this clear beat the saved best
    let mut game_over = false;
    let mut over_at: u64 = 0;

    let spin = 1_000_000u64;
    let mut acc: u64 = 0;
    let mut prev_t = now_ns();
    let mut next = now_ns();
    let mut intro_until = now_ns() + 1_600_000_000; // show the level-title card ~1.6s
    let mut level_start = now_ns(); // for the on-screen level timer
    let mut level_kibble0: u32 = 0; // kibble total at the level's start (for the tally)

    // A title card to open the session — any key starts, q quits, and it
    // auto-starts after a few seconds so it never blocks an unattended run.
    if !show_title_card(&mut out, &mut input, cols, rows) {
        drop(guard);
        return;
    }

    loop {
        if terminal::quit_requested() || input.quit {
            break;
        }
        input.poll();
        if input.quit || input.pressed(K_Q) {
            break;
        }
        if input.pressed(K_HELP) {
            help = !help;
            if help {
                // Tear down the live image so help text isn't hidden behind it.
                out.clear();
                backend.teardown(&mut out);
                let mut o = std::io::stdout().lock();
                let _ = o.write_all(&out);
                let _ = o.write_all(b"\x1b[2J");
                let _ = o.flush();
            } else {
                full_redraw = true;
            }
        } else if input.pressed(K_ESC) {
            if help {
                help = false; // Esc closes help rather than quitting
                full_redraw = true;
            } else {
                break;
            }
        }
        if input.pressed(K_P) && !help {
            paused = !paused;
            full_redraw = true;
        }
        if input.pressed(K_G) && !help {
            assist = !assist; // practice mode: invulnerability for learning a level
            full_redraw = true;
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

        // Throw a Sudsball (spacebar / 'c') — a non-violent bonk that pops critters.
        // Always available; the gear tier just makes it snappier and faster: Bubble
        // is the throw specialist, so picking up a Bubble Bone is felt immediately.
        if fire_cd > 0 {
            fire_cd -= 1;
        }
        if input.fire_pressed() && fire_cd == 0 {
            let dir = if sim.player.facing >= 0 { 1i8 } else { -1i8 };
            let px = sim.player.pos.x + sim.player.w / 2.0;
            let py = sim.player.pos.y + sim.player.h * 0.4;
            let (speed, size, cooldown) = match power {
                Power::Small => (2.6, 4.0, 24),
                Power::Big => (3.2, 4.0, 16),
                Power::Bubble => (4.2, 5.0, 9),
            };
            projectiles.push(Mob::new(px, py, size, size, dir, speed, Gait::Fly));
            fire_cd = cooldown;
        }

        // Dash ('x'): a quick burst in the facing direction with brief i-frames
        // (a dodge), on a cooldown. The burst itself runs in the physics block.
        if dash_cd > 0 {
            dash_cd -= 1;
        }
        if input.pressed(K_X) && dash_cd == 0 && !won && !game_over && !paused {
            dashing = DASH_FRAMES;
            dash_cd = DASH_CD;
            sim.player.vel.x = sim.player.facing as f64 * DASH_SPEED;
            invuln = invuln.max(DASH_FRAMES + 3); // dodge through contact
            let cl = sim.clock();
            sim.fx.spawn(&scamper::effects::DASH, sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y + sim.player.h * 0.4, cl);
            sim.fx.spawn_word(scamper::strings::t("fx.dash"), (200, 230, 255), sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y - 8.0, cl);
            shake.bump(0.25);
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
        // Munchii's hitbox tracks his gear (tiny when small, full when powered),
        // anchored at the feet so a power-up grows him upward off the ground.
        let (hw, hh) = power.hitbox();
        resize_player(&mut sim.player, hw, hh);
        if assist {
            invincible = invincible.max(3); // practice mode keeps the star shield up
        }
        // Zoomies Treat: a timed speed burst. Boost top speed + accel while it
        // lasts, and streak a dash trail behind Munchii (a delighter that also
        // signals the buff). Restores the base feel when it runs out.
        if zoomies > 0 {
            sim.fp.max_run = base_max_run * 1.6;
            sim.fp.run_accel = base_run_accel * 1.7;
            if sim.player.grounded && sim.player.vel.x.abs() > base_max_run && now.saturating_sub(dash_t) > 70 * 1_000_000 {
                let behind = sim.player.pos.x + sim.player.w / 2.0 - sim.player.facing as f64 * sim.player.w;
                sim.fx.spawn(&scamper::effects::DASH, behind, sim.player.pos.y + sim.player.h * 0.4, now);
                dash_t = now;
            }
        } else {
            sim.fp.max_run = base_max_run;
            sim.fp.run_accel = base_run_accel;
        }
        // A dash in progress overrides the cap so the burst isn't clamped away.
        if dashing > 0 {
            sim.fp.max_run = sim.fp.max_run.max(DASH_SPEED);
        }
        // Wall-slide friction sparks — surface the engine's wall-slide/jump in play.
        if sim.player.state == State::WallSliding && now.saturating_sub(wallspark_t) > 60 * 1_000_000 {
            let side = if sim.player.wall_dir > 0 { sim.player.w } else { 0.0 };
            sim.fx.spawn(&scamper::effects::SPARK, sim.player.pos.x + side, sim.player.pos.y + sim.player.h * 0.5, now);
            wallspark_t = now;
        }
        // Ambient particles: snowfall / bubbles / drifting leaves, by theme — a
        // living backdrop, sprinkled across the visible band around Munchii.
        if let Some(amb) = ambient_fx(world.theme) {
            if now.saturating_sub(ambient_t) > 200 * 1_000_000 {
                let half = fb_w as f64 / (2.0 * power.zoom() as f64);
                let r = (now / 1_000_000 % 1000) as f64 / 1000.0;
                let ax = sim.player.pos.x + sim.player.w / 2.0 - half + r * 2.0 * half;
                let ay = sim.player.pos.y - fb_h as f64 / power.zoom() as f64 * 0.35;
                sim.fx.spawn(amb, ax, ay, now);
                ambient_t = now;
            }
        }
        // Invincibility: a continuous sparkle halo so the Star state reads in every
        // tier (incl. mono B&W), even though Munchii himself isn't hidden.
        if invincible > 0 && now.saturating_sub(star_t) > 70 * 1_000_000 {
            let off = ((now / 5_000_000) % 9) as f64 - 4.0;
            sim.fx.spawn(&scamper::effects::SPARKLE, sim.player.pos.x + sim.player.w / 2.0 + off, sim.player.pos.y - 2.0 + off.abs() * 0.5, now);
            star_t = now;
        }
        // Bubble gear trails soap bubbles — a continuous aura so the Bubble state
        // reads distinctly from plain Big gear in every tier (incl. mono B&W).
        if power == Power::Bubble && now.saturating_sub(aura_t) > 130 * 1_000_000 {
            let bx = sim.player.pos.x + sim.player.w / 2.0 + ((now / 7_000_000) % 7) as f64 - 3.0;
            sim.fx.spawn(&scamper::effects::BUBBLE, bx, sim.player.pos.y - 2.0, now);
            aura_t = now;
        }
        // Glide feathers: a gentle wisp trail while actively floating.
        if glide && !sim.player.grounded && sim.player.vel.y > 0.0 && input.jump_held() && now.saturating_sub(glide_t) > 90 * 1_000_000 {
            let fx = sim.player.pos.x + sim.player.w / 2.0 + ((now / 5_000_000) % 5) as f64 - 2.0;
            sim.fx.spawn(&scamper::effects::FEATHER, fx, sim.player.pos.y + sim.player.h * 0.5, now);
            glide_t = now;
        }
        // Boss telegraph: an angry alert pops over Baron Whiskers as he paces, so
        // he reads as a live threat (and hints "jump his head").
        if !won && now.saturating_sub(boss_t) > 1100 * 1_000_000 {
            if let Some(b) = actors.iter().find(|a| a.kind == "baron_whiskers" && a.mob.alive) {
                sim.fx.spawn(&scamper::effects::BANG, b.mob.pos.x + b.mob.w / 2.0, b.mob.pos.y - 8.0, now);
                boss_t = now;
            }
        }
        if !won && !game_over && !help && !paused {
            acc += elapsed;
            while acc >= SIM_DT_NS {
                let inp = InputFrame {
                    axis_x: input.axis_x() as i8,
                    jump_pressed: pending_jump,
                    jump_held: input.jump_held(),
                    down_held: input.down_held(),
                };
                pending_jump = false;
                let pre_air = !sim.player.grounded;
                let pre_vy = sim.player.vel.y;
                sim.step(&world.map, inp);
                zoomies = zoomies.saturating_sub(1); // burst counts down per sim tick
                invincible = invincible.saturating_sub(1); // star counts down per tick
                dashing = dashing.saturating_sub(1); // dash burst window
                // Touchdown after a real fall → a dust scuff + a soft thump.
                if pre_air && sim.player.grounded && pre_vy > 220.0 {
                    sim.fx.spawn(&scamper::effects::DUST, sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y + sim.player.h, now);
                    shake.bump(0.2);
                }
                // Leaving the ground upward (a jump) → a little kick-off puff.
                if !pre_air && !sim.player.grounded && sim.player.vel.y < 0.0 {
                    sim.fx.spawn(&scamper::effects::DUST, sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y + sim.player.h, now);
                }
                if sim.player.grounded {
                    combo = 0; // touching down ends the air combo
                    // Standing on a crumbling plank starts it shaking.
                    let fcx = ((sim.player.pos.x + sim.player.w / 2.0) / TILE).floor() as i32;
                    let fcy = ((sim.player.pos.y + sim.player.h + 1.0) / TILE).floor() as i32;
                    world.touch_crumble(fcx, fcy);
                }
                world.tick_crumbles(); // advance shake → drop → regrow
                // Glide: holding jump while falling (with the Flutter Collar) caps
                // the descent to a gentle float.
                if glide && !sim.player.grounded && inp.jump_held && sim.player.vel.y > GLIDE_FALL {
                    sim.player.vel.y = GLIDE_FALL;
                }
                // Head-bonk a block from below: questions/coin-blocks cough up an
                // item, breakable bricks shatter.
                if sim.player.bonked_head {
                    let cy = ((sim.player.pos.y - 1.0) / TILE).floor() as i32;
                    // Bonk forgivingly across the whole head width (the hitbox is
                    // narrow; pick the first bonkable column, else the center).
                    let lx = (sim.player.pos.x / TILE).floor() as i32;
                    let rx = ((sim.player.pos.x + sim.player.w - 0.01) / TILE).floor() as i32;
                    let center = ((sim.player.pos.x + sim.player.w / 2.0) / TILE).floor() as i32;
                    let cx = (lx..=rx).find(|&c| world.is_block(c, cy)).unwrap_or(center);
                    let now = sim.clock();
                    let (bxw, byw) = (cx as f64 * TILE + TILE / 2.0, cy as f64 * TILE);
                    let dropped = match world.bonk(cx, cy) {
                        // A brick shatters (scuff burst) and may drop a coin.
                        Bonk::Broke(item) => {
                            sim.fx.spawn(&scamper::effects::BONK, bxw, byw, now);
                            sim.fx.spawn_word(scamper::strings::t("fx.bonk"), (255, 236, 150), bxw, byw - 6.0, now);
                            shake.bump(0.35); // a little crunch when a brick shatters
                            item
                        }
                        // A question block coughs up its contents and stays.
                        Bonk::Released(item) => Some(item),
                        // Hit a solid that won't budge → a startled exclamation.
                        Bonk::Nothing => {
                            let hx = sim.player.pos.x + sim.player.w / 2.0;
                            sim.fx.spawn(&scamper::effects::BANG, hx, sim.player.pos.y - 6.0, now);
                            sim.fx.spawn_word(scamper::strings::t("fx.woah"), (255, 232, 120), hx, sim.player.pos.y - 10.0, now);
                            None
                        }
                    };
                    if let Some(item) = dropped {
                        if item == "kibble" {
                            // A coin leaps out of the top of the block and is banked.
                            sim.fx.spawn(&scamper::effects::COIN, bxw, byw - TILE, now);
                            kibble += 1;
                        } else {
                            sim.fx.spawn(&scamper::effects::SPARKLE, bxw, byw, now);
                            // pop the power-up out onto the block top to grab
                            let m = Mob::new(cx as f64 * TILE, (cy - 1) as f64 * TILE, 12.0, 12.0, 1, 0.0, Gait::Still);
                            actors.push(Actor { mob: m, kind: item, item: true, mode: Mode::Walk, alerted: false });
                        }
                    }
                    full_redraw = true;
                }
                // Skid dust: reversing direction at speed kicks up a scuff (throttled).
                if skid_cd > 0 {
                    skid_cd -= 1;
                }
                if skid_cd == 0 && sim.player.grounded && inp.axis_x != 0 && (inp.axis_x as f64) * sim.player.vel.x < -1.0 && sim.player.vel.x.abs() > 70.0 {
                    sim.fx.spawn(&scamper::effects::DUST, sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y + sim.player.h, sim.clock());
                    skid_cd = 9;
                }
                // Snapshot lift positions so we can carry the player by their delta.
                let lifts_pre: Vec<(usize, f64)> = actors.iter().enumerate().filter(|(_, a)| a.kind == "lift").map(|(i, a)| (i, a.mob.pos.x)).collect();
                // Step creatures/items and resolve pounces, pickups, and hits.
                let hits = step_actors(&mut actors, &world.map, &mut sim.player, &mut kibble, &mut power, invincible > 0);
                // Ride moving platforms: land on a lift's top and get carried along.
                for (i, px0) in &lifts_pre {
                    let m = &actors[*i].mob;
                    sim.player.ride_platform(m.pos.x, m.pos.y, m.w, m.h, m.pos.x - px0);
                }
                let fxclock = sim.clock();
                for &(e, x, y) in &hits.fx {
                    sim.fx.spawn(e, x, y, fxclock); // pops / pickups feedback (kind-specific burst)
                }
                // A "POP!" shout + little shake for any critter popped this step.
                if hits.pops > 0 {
                    let (pxw, pyw) = hits.pop_xy;
                    sim.fx.spawn_word(scamper::strings::t("fx.pop"), (255, 236, 150), pxw, pyw - 6.0, fxclock);
                    shake.bump(0.25);
                }
                // Air combo: chaining pounces before landing escalates the reward.
                // Each pounce past the first banks a bonus kibble and shouts the tally.
                if hits.pounces > 0 {
                    for _ in 0..hits.pounces {
                        combo += 1;
                        if combo >= 2 {
                            kibble += combo; // bonus grows with the chain
                            sim.fx.spawn_word(combo_word(combo), (255, 200, 120), sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y - 10.0, fxclock);
                            shake.bump(0.2);
                        }
                    }
                }
                step_projectiles(&mut projectiles, &mut actors, &world.map, &mut kibble);
                emit_thrower_sticks(&actors, &mut hostiles, &sim.player);
                let stick_hit = step_hostiles(&mut hostiles, &world.map, &sim.player);
                let mut got_1up = hits.oneup;
                if hits.oneup {
                    lives += 1;
                }
                while kibble >= next_1up {
                    lives += 1; // 100 kibble = an extra life
                    next_1up += 100;
                    got_1up = true;
                }
                if got_1up {
                    sim.fx.spawn_word(scamper::strings::t("fx.oneup"), (140, 240, 150), sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y - 8.0, fxclock);
                }
                if hits.zoomies {
                    zoomies = ZOOMIES_TICKS; // (re)charge the speed burst
                    sim.fx.spawn_word(scamper::strings::t("fx.wahoo"), (255, 210, 90), sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y - 8.0, fxclock);
                }
                if hits.collar {
                    glide = true; // unlock gliding
                    sim.fx.spawn_word(scamper::strings::t("fx.float"), (180, 230, 255), sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y - 8.0, fxclock);
                }
                if hits.star {
                    invincible = STAR_TICKS;
                    sim.fx.spawn_word(scamper::strings::t("fx.star"), (255, 240, 120), sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y - 8.0, fxclock);
                }
                if hits.bounced {
                    sim.fx.spawn_word(scamper::strings::t("fx.boing"), (160, 240, 200), sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y - 8.0, fxclock);
                    shake.bump(0.3);
                }
                boss_cd = boss_cd.saturating_sub(1);
                if hits.boss_hit && boss_cd == 0 {
                    boss_hp -= 1;
                    boss_cd = 45; // brief invulnerability between pounces
                    shake.bump(0.6);
                    if boss_hp > 0 {
                        // He's hurt and angrier — pace faster.
                        for a in actors.iter_mut() {
                            if a.kind == "baron_whiskers" {
                                a.mob.speed += 0.35;
                            }
                        }
                        sim.fx.spawn_word(scamper::strings::t("fx.ow"), (255, 150, 150), sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y - 8.0, fxclock);
                    } else if !won {
                        // Defeated — the bath drains and the level is won.
                        for a in actors.iter_mut() {
                            if a.kind == "baron_whiskers" {
                                a.mob.alive = false;
                            }
                        }
                        won = true;
                        won_at = now;
                        sim.fx.spawn(&scamper::effects::CHEER, sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y, fxclock);
                        sim.fx.spawn_word(scamper::strings::t("fx.wahoo"), (255, 230, 140), sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y - 12.0, fxclock);
                    }
                }
                if hits.plug && !won {
                    won = true; // pulled the bath plug → level complete
                    won_at = now;
                    sim.fx.spawn(&scamper::effects::CHEER, sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y, sim.clock());
                }
                if invuln > 0 {
                    invuln -= 1;
                } else if (hits.hurt || stick_hit) && invincible == 0 {
                    if power == Power::Small {
                        lives -= 1; // wipeout → lose a life, back to the last checkpoint
                        sim = sim_at(respawn);
                    } else {
                        power = power.dropped(); // shed a tier of gear
                    }
                    invuln = 90; // ~1.5s of grace so you don't re-hit instantly
                    glide = false; // a hit knocks the collar loose
                    shake.bump(0.85); // a hard jolt on taking a hit
                    full_redraw = true;
                }
                if lives < 0 {
                    game_over = true;
                    over_at = now;
                }
                acc -= SIM_DT_NS;
            }
        } else {
            acc = 0;
        }
        if game_over && now.saturating_sub(over_at) >= 1600 * 1_000_000 {
            break; // GAME OVER shown — back to the menu
        }

        let (px, py, pw_, ph_) = (sim.player.pos.x, sim.player.pos.y, sim.player.w, sim.player.h);
        // hazard (lava/water) or a fall into the pit below the level → lose a life.
        if !won && !game_over && (world.hazard_overlap(px, py, pw_, ph_) || py > world.px_h() + TILE) {
            lives -= 1;
            if lives < 0 {
                game_over = true;
                over_at = now;
            } else {
                sim = sim_at(respawn);
                power = Power::Small;
                invuln = 90;
            }
            full_redraw = true;
        }
        // Advance the respawn point as Munchii crosses each checkpoint (a flag-raise
        // sparkle marks it). One-way: passing right of a checkpoint banks it.
        while next_cp < world.checkpoints.len() && sim.player.pos.x + sim.player.w / 2.0 >= world.checkpoints[next_cp].0 {
            respawn = world.checkpoints[next_cp];
            sim.fx.spawn(&scamper::effects::SPARKLE, respawn.0, respawn.1 - TILE, now);
            next_cp += 1;
        }
        // goal reached → level complete; after a short beat, auto-advance to the
        // next sibling level (a debugging aid: walk the whole set without quitting).
        if !won {
            if let Some((gx, _)) = world.goal {
                if px + pw_ / 2.0 >= gx {
                    won = true;
                    won_at = now;
                    sim.fx.spawn(&scamper::effects::CHEER, sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y, sim.clock());
                }
            }
        } else if now.saturating_sub(won_at) >= 1700 * 1_000_000 {
            if let Some((next, lvl)) = next_level_path(&cur_path).and_then(|p| load_level_file(&p).ok().map(|l| (p, l))) {
                cur_path = next;
                level = lvl;
                world = LevelWorld::from_level(&level);
                sim = sim_at(world.spawn);
                respawn = world.spawn;
                next_cp = 0;
                actors = build_actors(&world);
                projectiles.clear();
                hostiles.clear();
                power = Power::Small;
                play_bgm(&level);
                won = false;
                boss_hp = 3;
                boss_cd = 0;
                invincible = 0;
                celebrated = false;
                is_new_best = false;
                intro_until = now + 1_600_000_000;
                level_start = now;
                level_kibble0 = kibble;
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
                    respawn = spawn;
                    next_cp = 0;
                    actors = build_actors(&world);
                    projectiles.clear();
                    hostiles.clear();
                    power = Power::Small;
                    play_bgm(&level);
                    boss_hp = 3;
                    boss_cd = 0;
                    invincible = 0;
                    celebrated = false;
                    is_new_best = false;
                    intro_until = now + 1_600_000_000;
                    level_start = now;
                    level_kibble0 = kibble;
                    full_redraw = true;
                }
            }
        }

        if help {
            // Controls overlay (frozen): plain adaptive text, fits any window.
            render_play_help(&mut out, backend.name());
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.flush();
        } else {
            // Win flourish (once, the instant the level is cleared): a spray of
            // cheers across the view + a "CLEAR!" shout + a celebratory shake.
            if won && !celebrated {
                celebrated = true;
                let (cxw, cyw) = (sim.player.pos.x + sim.player.w / 2.0, sim.player.pos.y);
                let cl = sim.clock();
                for (dx, dy) in [(-22.0, -4.0), (0.0, -14.0), (22.0, -4.0), (-10.0, 4.0), (12.0, 6.0)] {
                    sim.fx.spawn(&scamper::effects::CHEER, cxw + dx, cyw + dy, cl);
                }
                sim.fx.spawn_word(scamper::strings::t("fx.clear"), (255, 240, 150), cxw, cyw - 16.0, cl);
                shake.bump(0.5);
                // Record the clear time; flag (and persist) a new personal best.
                let fsec = (won_at.saturating_sub(level_start) / 1_000_000_000) as u32;
                is_new_best = bests.get(&level.id).map_or(true, |&b| fsec < b);
                if is_new_best {
                    bests.insert(level.id.clone(), fsec);
                    save_bests(&bests);
                }
            }
            // --- render the camera window (shared with the headless soak harness) ---
            let shake_off = shake.offset(frame_ix, 7.0);
            if shake.active() {
                full_redraw = true; // a moving camera needs the whole scene repainted
            }
            // Camera look-ahead: ease the view toward the way Munchii is moving so
            // there's more room to see what's coming (only when he's actually moving).
            let look_target = if sim.player.vel.x.abs() > 30.0 { sim.player.facing as f64 * 34.0 } else { 0.0 };
            look_ahead += (look_target - look_ahead) * 0.08;
            frame_ix += 1;
            // Post-hit i-frames: flicker Munchii (hide on alternate ~4-frame beats).
            let blink = invuln > 0 && (frame_ix / 4) % 2 == 0;
            if invuln > 0 {
                full_redraw = true; // blinking sprite needs a clean repaint each frame
            }
            draw_play_frame(&mut fb, backend.as_mut(), &mut out, &world, &sim, &actors, &projectiles, &hostiles, fb_w, fb_h, cols, rows, full_redraw, input.down_held(), power.zoom(), shake_off, blink, look_ahead);
            full_redraw = false;
            let secs = ((if won { won_at } else { now }).saturating_sub(level_start) / 1_000_000_000) as u32;
            render_play_status(&mut status, &level, sim.player.state, backend.name(), won, game_over, kibble, lives, power, zoomies > 0, glide, invincible > 0, secs, rows + 1, cols);
            // Slim level-progress bar on the top row: a marker rides ─── toward a ⚑
            // at the level's end. (Skipped during cards so they aren't cluttered.)
            if !won && !game_over && !paused && now >= intro_until {
                use std::fmt::Write;
                let bw = (cols as usize).saturating_sub(6).max(4);
                let frac = (sim.player.pos.x / world.px_w().max(1.0)).clamp(0.0, 1.0);
                let pos = ((frac * (bw - 1) as f64) as usize).min(bw - 1);
                let mut bar = String::new();
                for i in 0..bw {
                    bar.push(if i == pos { '◆' } else { '─' });
                }
                let _ = write!(status, "\x1b[1;3H\x1b[2m{bar}\x1b[0m\x1b[1;{}H⚑", bw + 3);
            }
            if assist && !won && !game_over {
                use std::fmt::Write;
                let _ = write!(status, "\x1b[1;{}H\x1b[1;7m ⛑ ASSIST \x1b[0m", (cols as usize).saturating_sub(10).max(1));
            }
            // Boss health bar: only while Baron Whiskers is present and the fight's on.
            if !won && !game_over && actors.iter().any(|a| a.kind == "baron_whiskers" && a.mob.alive) {
                let pips: String = (0..3).map(|i| if (i as i32) < boss_hp { '♥' } else { '·' }).collect();
                let bar = format!(" BARON WHISKERS  {pips} ");
                scamper::ui::center_card(&mut status, cols, 2, &[&bar], true);
            }
            if paused {
                status.clear();
                scamper::ui::status_line(&mut status, rows + 1, "⏸ PAUSED — p resume · q quit");
            }
            // Level-title card: a centered banner for the first ~1.6s of a level,
            // with your best clear-time as a target if you've cleared it before.
            if now < intro_until {
                let card = format!(" {} — {} ", level.id, level.theme);
                let mut lines: Vec<&str> = vec![&card];
                let best_line;
                if let Some(&b) = bests.get(&level.id) {
                    best_line = format!(" best {}:{:02} ", b / 60, b % 60);
                    lines.push(&best_line);
                }
                scamper::ui::center_card(&mut status, cols, (rows / 2).max(1), &lines, true);
            }
            // Game-over card during the brief hold before returning to the menu.
            if game_over {
                let l0 = "  ✗  GAME OVER  ✗  ".to_string();
                let l1 = format!("  kibble {kibble}  ·  back to menu…  ");
                scamper::ui::center_card(&mut status, cols, (rows / 2).saturating_sub(1).max(1), &[&l0, &l1], true);
                full_redraw = true;
            }
            // Results card on level clear: kibble collected, time, and a star rating
            // (faster = more stars). Shown during the win pause before auto-advance.
            if won && !game_over {
                let stars = if secs < 25 { "★★★" } else if secs < 55 { "★★ " } else { "★  " };
                let best = bests.get(&level.id).copied().unwrap_or(secs);
                let l0 = format!("  ✦  {}  CLEAR!  ✦  ", level.id);
                let l1 = format!("  kibble +{}   time {}:{:02}  ", kibble.saturating_sub(level_kibble0), secs / 60, secs % 60);
                let l2 = format!("  rating {}  ", stars);
                let l3 = if is_new_best {
                    "  ☆ NEW BEST! ☆  ".to_string()
                } else {
                    format!("  best {}:{:02}  ", best / 60, best % 60)
                };
                scamper::ui::center_card(&mut status, cols, (rows / 2).saturating_sub(2).max(1), &[&l0, &l1, &l2, &l3], true);
                full_redraw = true;
            }
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.write_all(status.as_bytes());
            let _ = o.flush();
            if now < intro_until {
                full_redraw = true; // repaint to clear the card once it expires
            }
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

/// Munchii's power tier (gear, not damage): small → big (took a Big Kibble) →
/// bubble (a Bubble Bone — can lob Sudsballs). A hit drops one tier; a hit while
/// small is a wipeout (respawn). Mirrors the classic three-tier model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Power {
    Small,
    Big,
    Bubble,
}

impl Power {
    /// The tier after taking a hit (Bubble→Big→Small).
    fn dropped(self) -> Power {
        match self {
            Power::Bubble => Power::Big,
            _ => Power::Small,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Power::Small => "small",
            Power::Big => "big",
            Power::Bubble => "bubble",
        }
    }
    /// How much the *environment* is magnified while wearing this gear. Tiny
    /// Munchii is a flea in a 4× world; powering up snaps the tiles down to 1× so
    /// he's suddenly the big one — the whole twist, with no character resize.
    fn zoom(self) -> usize {
        match self {
            Power::Small => 4,
            _ => 1,
        }
    }
    /// Munchii's hitbox (world px) for this gear: it shrinks with the world (base
    /// ÷ zoom) so his *on-screen* size stays constant. Physics is untouched, so a
    /// jump clears the same number of tiles whatever size he is.
    fn hitbox(self) -> (f64, f64) {
        let z = self.zoom() as f64;
        (BODY_W / z, BODY_H / z)
    }
}

/// Munchii's base hitbox (the powered "big" size, = `Player::new`'s default).
const BODY_W: f64 = 12.0;
const BODY_H: f64 = 16.0;

/// Resize Munchii's hitbox to `(w, h)`, keeping his feet planted and horizontally
/// centered (so growing on a power-up lifts him off the ground rather than
/// sinking him into it). A no-op when already that size.
fn resize_player(p: &mut Player, w: f64, h: f64) {
    if (p.w - w).abs() < 1e-6 && (p.h - h).abs() < 1e-6 {
        return;
    }
    let center = p.pos.x + p.w / 2.0;
    let feet = p.pos.y + p.h;
    p.w = w;
    p.h = h;
    p.pos.x = center - w / 2.0;
    p.pos.y = feet - h;
}

/// A rollo/hardhat's curl state: walking, a still curled ball, or a kicked ball
/// rolling fast (which pops other critters).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Walk,
    ShellStill,
    ShellRoll,
}

/// A live creature or item in the level: an engine [`Mob`] plus the game meaning
/// (its sprite id, whether it's a collectible, and its curl mode). Built from the
/// level's IR entities; stepped and collided each tick.
struct Actor {
    mob: Mob,
    kind: String,
    item: bool,
    mode: Mode,
    alerted: bool, // a chaser that has spotted Munchii (edge-triggers the "!" cue)
}

/// Item kinds collect on touch; everything else is a creature you pounce.
fn is_item(kind: &str) -> bool {
    matches!(kind, "kibble" | "big_kibble" | "bubble_bone" | "zoomies_treat" | "lucky_squeaky" | "flutter_collar" | "star_bone")
}

/// Creatures that curl into a kickable ball when pounced (instead of popping).
fn is_curler(kind: &str) -> bool {
    matches!(kind, "rollo" | "rollo_sun" | "hardhat")
}
/// Spiky critters that can't be pounced (a stomp lands on the spines and hurts);
/// only a Sudsball pops them.
fn is_spiky(kind: &str) -> bool {
    matches!(kind, "prickle" | "prickle_sun")
}

/// The burst effect a popped critter leaves, by kind — so deaths read distinctly:
/// spiky things shatter into shards, flyers shed feathers, springers puff, and
/// everything else gives the bright BOP.
fn pop_fx(kind: &str) -> &'static scamper::effects::Effect {
    if is_spiky(kind) {
        &scamper::effects::SHARDS
    } else if is_flyer(kind) {
        &scamper::effects::FEATHER
    } else if kind == "springer" {
        &scamper::effects::PUFF
    } else {
        &scamper::effects::BOP
    }
}

/// The ambient particle that drifts through a theme (snowfall, bubbles, leaves),
/// or none for the bare themes. Spawned continuously for a living backdrop.
fn ambient_fx(theme: scamper::level::Theme) -> Option<&'static scamper::effects::Effect> {
    use scamper::level::Theme;
    match theme {
        Theme::Snow => Some(&scamper::effects::SNOW),
        Theme::Underwater => Some(&scamper::effects::BUBBLE),
        Theme::Overworld => Some(&scamper::effects::LEAF),
        _ => None,
    }
}

/// Air-combo shout for an `n`-pounce chain (static so it can float as a word pop).
fn combo_word(n: u32) -> &'static str {
    match n {
        0 | 1 | 2 => "COMBO x2",
        3 => "COMBO x3",
        4 => "COMBO x4",
        5 => "COMBO x5",
        6 => "COMBO x6",
        _ => "COMBO MAX!",
    }
}

/// Speed of a kicked shell, px/tick.
const SHELL_SPEED: f64 = 2.6;
/// Chaser AI: how far it can "see" Munchii, and its charge vs idle-patrol speeds.
const CHASE_RANGE: f64 = 150.0;
const CHASE_SPEED: f64 = 1.4;
const CHASE_IDLE: f64 = 0.35;

/// The background track for a level's theme. There's no audio engine yet, so this
/// is the hook: it names the track and logs it on level load (wire to real audio
/// later). Themes map to the campaign's romp / dig / splash / bath / chill moods.
fn bgm_for(theme: &str) -> &'static str {
    match Theme::from_str(theme) {
        Theme::Underground => "dig",
        Theme::Underwater => "splash",
        Theme::Castle => "bath-house",
        Theme::Snow => "chill",
        Theme::Overworld => "romp",
    }
}

/// Play (hook) the track for a level — logs it for now.
fn play_bgm(level: &Level) {
    dlog!("bgm: {} (theme {})", bgm_for(&level.theme), level.theme);
}

/// Air & water creatures cruise on the `Fly` gait (no gravity, bob along a line).
fn is_flyer(kind: &str) -> bool {
    matches!(kind, "flutterbug" | "zoomdisc" | "sudsfish" | "sudsfish_sun" | "puffer" | "pop" | "drip")
}

/// Game design: pick a gait/size for an entity kind. `_sun` (red) ground variants
/// stay on ledges; air/water kinds fly; the rest wander.
fn gait_for(kind: &str, item: bool) -> (Gait, f64, f64, f64) {
    if item || kind == "bath_plug" || kind == "rescued_pup" {
        (Gait::Still, 0.0, 12.0, 12.0) // inert: items, the plug, a rescued pup
    } else if kind == "baron_whiskers" {
        (Gait::Wander, 0.5, 22.0, 26.0) // the boss: a big box that paces the ledge
    } else if kind == "dandi" || kind == "dandi_sun" {
        (Gait::Bob, 0.0, 12.0, 14.0) // snapping dandelion: rises/lowers from a pipe
    } else if kind == "chaser" {
        (Gait::Wander, CHASE_IDLE, 13.0, 13.0) // pursues Munchii on sight (AI in step_actors)
    } else if kind == "springer" {
        (Gait::Hop, 0.5, 12.0, 12.0) // a bouncing critter — time your pounce mid-hop
    } else if kind == "lift" {
        (Gait::Bob, 0.0, 28.0, 6.0) // a vertical elevator platform — ride it up/down
    } else if kind == "trampoline" {
        (Gait::Still, 0.0, 16.0, 8.0) // a bounce pad — inert, launches you on landing
    } else if kind == "swooper" {
        (Gait::Swoop, 0.7, 12.0, 12.0) // a moth that weaves through the air
    } else if kind == "hardhat" {
        (Gait::Careful, 0.4, 12.0, 14.0) // hard-hat acorn: stays on its ledge
    } else if is_flyer(kind) {
        (Gait::Fly, 0.7, 12.0, 12.0)
    } else if kind.ends_with("_sun") {
        (Gait::Careful, 0.45, 12.0, 14.0)
    } else {
        (Gait::Wander, 0.45, 12.0, 14.0)
    }
}

/// Build the live actor list from a world's entities.
fn build_actors(world: &LevelWorld) -> Vec<Actor> {
    world
        .ents
        .iter()
        .filter(|e| scamper::sprite::get(&e.kind).is_some())
        .map(|e| {
            let item = is_item(&e.kind);
            let (gait, speed, w, h) = gait_for(&e.kind, item);
            Actor { mob: Mob::new(e.cx as f64 * TILE, e.cy as f64 * TILE, w, h, -1, speed, gait), kind: e.kind.clone(), item, mode: Mode::Walk, alerted: false }
        })
        .collect()
}

/// Step Sudsball projectiles: fly, expire on a wall or after their lifetime, and
/// pop any creature they touch (into a treat). Dead projectiles are reaped.
fn step_projectiles(projectiles: &mut Vec<Mob>, actors: &mut [Actor], map: &TileMap, kibble: &mut u32) {
    for p in projectiles.iter_mut() {
        p.step(map);
        if p.blocked || p.age > 90 {
            p.alive = false;
            continue;
        }
        for a in actors.iter_mut() {
            if a.mob.alive && !a.item && aabb_overlap(p.pos.x, p.pos.y, p.w, p.h, a.mob.pos.x, a.mob.pos.y, a.mob.w, a.mob.h) {
                a.mob.alive = false;
                *kibble += 2;
                p.alive = false;
                break;
            }
        }
    }
    projectiles.retain(|p| p.alive);
}

/// Stick Squirrels lob arcing sticks at Munchii. On a cadence, each on-screen-ish
/// thrower spawns a hostile Ballistic stick aimed at the player.
fn emit_thrower_sticks(actors: &[Actor], hostiles: &mut Vec<Mob>, player: &Player) {
    for a in actors {
        if a.kind == "stick_squirrel" && a.mob.alive && a.mob.age % 96 == 48
            && (a.mob.pos.x - player.pos.x).abs() < 220.0
        {
            let dir = if player.pos.x >= a.mob.pos.x { 1.0 } else { -1.0 };
            let mut s = Mob::new(a.mob.pos.x + a.mob.w / 2.0, a.mob.pos.y, 4.0, 4.0, dir as i8, 0.0, Gait::Ballistic);
            s.vel = Vec2::new(dir * 1.6, -3.2);
            hostiles.push(s);
        }
    }
}

/// Step hostile projectiles (thrown sticks): fly, expire on a wall or lifetime,
/// and report a hit if one touches the player.
fn step_hostiles(hostiles: &mut Vec<Mob>, map: &TileMap, player: &Player) -> bool {
    let mut hurt = false;
    for h in hostiles.iter_mut() {
        h.step(map);
        if h.blocked || h.age > 150 {
            h.alive = false;
            continue;
        }
        if aabb_overlap(player.pos.x, player.pos.y, player.w, player.h, h.pos.x, h.pos.y, h.w, h.h) {
            hurt = true;
            h.alive = false;
        }
    }
    hostiles.retain(|h| h.alive);
    hurt
}

/// Upward velocity (px/s) from a successful pounce — a little hop (~0.6× a jump).
const POUNCE_BOUNCE: f64 = -220.0;
/// Upward launch from a trampoline — much higher than a jump (clears tall gaps).
const TRAMPO_LAUNCH: f64 = -560.0;

/// Step every actor one tick and resolve player↔actor collisions. Returns true if
/// the player took a non-pounce creature hit. On a pounce the creature pops (into
/// a treat) and the player gets a bounce; items collect into `kibble`.
/// What the player touched this tick.
#[derive(Default)]
struct Hits {
    hurt: bool,    // a non-pounce creature touch
    plug: bool,    // pulled the bath plug → win
    oneup: bool,   // collected a Lucky Squeaky → extra life
    zoomies: bool, // collected a Zoomies Treat → timed speed burst
    collar: bool,  // collected a Flutter Collar → unlocks gliding
    pounces: u32,  // creatures pounced this step (drives the air combo)
    pops: u32,     // critters popped this step, by any means (drives the POP! cue)
    pop_xy: (f64, f64), // a representative pop location (for the cue)
    bounced: bool, // landed on a trampoline → launched high
    boss_hit: bool, // pounced the boss this step (gated by i-frames in run_play)
    star: bool,     // collected a Star Bone → invincibility
    /// Effects to spawn (clip, world center-x, world top-y) for pops / pickups.
    fx: Vec<(&'static scamper::effects::Effect, f64, f64)>,
}

fn step_actors(actors: &mut [Actor], map: &TileMap, player: &mut Player, kibble: &mut u32, power: &mut Power, invincible: bool) -> Hits {
    let mut hits = Hits::default();
    // Reactive AI (game logic, so the engine Mob stays dumb): a Chaser locks onto
    // Munchii and charges when he's within sight on roughly the same level; idles
    // into a slow patrol otherwise. The first frame it spots him pops a "!" cue.
    for a in actors.iter_mut() {
        if a.kind == "chaser" && a.mob.alive {
            let dx = player.pos.x - a.mob.pos.x;
            let spotted = dx.abs() < CHASE_RANGE && (player.pos.y - a.mob.pos.y).abs() < 40.0;
            if spotted {
                if !a.alerted {
                    hits.fx.push((&scamper::effects::BANG, a.mob.pos.x + a.mob.w / 2.0, a.mob.pos.y - 6.0));
                }
                a.alerted = true;
                a.mob.facing = if dx >= 0.0 { 1 } else { -1 };
                a.mob.speed = CHASE_SPEED;
            } else {
                a.alerted = false;
                a.mob.speed = CHASE_IDLE;
            }
        }
    }
    for a in actors.iter_mut() {
        a.mob.step(map);
    }

    // A rolling shell pops any normal creature it overlaps (gathered first to dodge
    // overlapping mutable borrows).
    let shells: Vec<(f64, f64, f64, f64)> = actors
        .iter()
        .filter(|a| a.mode == Mode::ShellRoll && a.mob.alive)
        .map(|a| (a.mob.pos.x, a.mob.pos.y, a.mob.w, a.mob.h))
        .collect();
    if !shells.is_empty() {
        for a in actors.iter_mut() {
            if a.mob.alive && !a.item && a.mode == Mode::Walk {
                let (bx, by, bw, bh) = (a.mob.pos.x, a.mob.pos.y, a.mob.w, a.mob.h);
                if shells.iter().any(|&(sx, sy, sw, sh)| aabb_overlap(sx, sy, sw, sh, bx, by, bw, bh)) {
                    a.mob.alive = false;
                    *kibble += 2;
                    hits.pops += 1;
                    hits.pop_xy = (bx + bw / 2.0, by);
                    hits.fx.push((pop_fx(&a.kind), bx + bw / 2.0, by));
                }
            }
        }
    }

    let (px, py, pw, ph, pvy) = (player.pos.x, player.pos.y, player.w, player.h, player.vel.y);
    for a in actors.iter_mut() {
        if !a.mob.alive {
            continue;
        }
        let (bx, by, bw, bh) = (a.mob.pos.x, a.mob.pos.y, a.mob.w, a.mob.h);
        if !aabb_overlap(px, py, pw, ph, bx, by, bw, bh) {
            continue;
        }
        // Invincibility star: bulldoze any ordinary critter on contact (the boss
        // and ridden props are exempt; items still collect below).
        if invincible && !a.item && !matches!(a.kind.as_str(), "baron_whiskers" | "lift" | "trampoline") {
            a.mob.alive = false;
            *kibble += 2;
            hits.pops += 1;
            hits.pop_xy = (bx + bw / 2.0, by);
            hits.fx.push((pop_fx(&a.kind), bx + bw / 2.0, by));
            continue;
        }
        let stomped = stomp(px, py, pw, ph, pvy, bx, by, bw, bh);
        // Curlers (rollo / hardhat): pounce curls them into a kickable shell.
        if is_curler(&a.kind) {
            match a.mode {
                Mode::Walk => {
                    if stomped {
                        a.mode = Mode::ShellStill;
                        a.mob.gait = Gait::Still;
                        a.mob.speed = 0.0;
                        player.vel.y = POUNCE_BOUNCE;
                        hits.pounces += 1;
                    } else {
                        hits.hurt = true;
                    }
                }
                Mode::ShellStill => {
                    if stomped {
                        player.vel.y = POUNCE_BOUNCE; // tap it again, stays put
                    } else {
                        // kick it away from Munchii
                        let dir: i8 = if px + pw / 2.0 <= bx + bw / 2.0 { 1 } else { -1 };
                        a.mode = Mode::ShellRoll;
                        a.mob.gait = Gait::Wander;
                        a.mob.speed = SHELL_SPEED;
                        a.mob.facing = dir;
                    }
                }
                Mode::ShellRoll => {
                    if stomped {
                        a.mode = Mode::ShellStill; // stomp stops a rolling shell
                        a.mob.gait = Gait::Still;
                        a.mob.speed = 0.0;
                        player.vel.y = POUNCE_BOUNCE;
                    } else {
                        hits.hurt = true; // a rolling shell bonks you
                    }
                }
            }
            continue;
        }
        match a.kind.as_str() {
            // Pull the plug → the bath drains and Baron Whiskers drops in: level won.
            "bath_plug" => hits.plug = true,
            // The boss: pounce his head to damage him (run_play tracks his health
            // and the between-hit i-frames); a side touch still hurts.
            "baron_whiskers" => {
                if stomp(px, py, pw, ph, pvy, bx, by, bw, bh) {
                    player.vel.y = POUNCE_BOUNCE;
                    hits.boss_hit = true;
                    hits.fx.push((&scamper::effects::BOP, bx + bw / 2.0, by));
                } else {
                    hits.hurt = true;
                }
            }
            // A lift is ridden (resolved in run_play), never fought — ignore contact.
            "lift" => {}
            // Trampoline: landing on it from above launches Munchii sky-high.
            "trampoline" => {
                if stomp(px, py, pw, ph, pvy, bx, by, bw, bh) {
                    player.vel.y = TRAMPO_LAUNCH;
                    hits.bounced = true;
                }
            }
            // Big Kibble grows a small Munchii; Bubble Bone gears up; else treats.
            "big_kibble" if a.item => {
                a.mob.alive = false;
                *power = if *power == Power::Small { Power::Big } else { *power };
                hits.fx.push((&scamper::effects::SPARKLE, bx + bw / 2.0, by));
            }
            "bubble_bone" if a.item => {
                a.mob.alive = false;
                *power = Power::Bubble;
                hits.fx.push((&scamper::effects::SPARKLE, bx + bw / 2.0, by));
            }
            "lucky_squeaky" if a.item => {
                a.mob.alive = false;
                hits.oneup = true; // extra life
                hits.fx.push((&scamper::effects::SPARKLE, bx + bw / 2.0, by));
            }
            "zoomies_treat" if a.item => {
                a.mob.alive = false;
                hits.zoomies = true; // timed speed burst
                *kibble += 1;
                hits.fx.push((&scamper::effects::SPARKLE, bx + bw / 2.0, by));
            }
            "flutter_collar" if a.item => {
                a.mob.alive = false;
                hits.collar = true; // unlocks gliding (hold jump while falling)
                hits.fx.push((&scamper::effects::SPARKLE, bx + bw / 2.0, by));
            }
            "star_bone" if a.item => {
                a.mob.alive = false;
                hits.star = true; // invincibility burst
                hits.fx.push((&scamper::effects::SPARKLE, bx + bw / 2.0, by));
            }
            _ if a.item => {
                a.mob.alive = false;
                *kibble += 1;
                // A loose kibble pops a coin like a coin-block does (consistent feedback).
                let fx = if a.kind == "kibble" { &scamper::effects::COIN } else { &scamper::effects::SPARKLE };
                hits.fx.push((fx, bx + bw / 2.0, by));
            }
            // Spiky critters can't be pounced — landing on the spines hurts. The
            // only safe answer is a Sudsball (see step_projectiles), so they teach
            // the throw. Any contact hurts.
            _ if is_spiky(&a.kind) => {
                hits.hurt = true;
            }
            // A normal creature: pounce pops it into a treat; a side touch hurts.
            _ => {
                if stomp(px, py, pw, ph, pvy, bx, by, bw, bh) {
                    a.mob.alive = false;
                    *kibble += 2;
                    player.vel.y = POUNCE_BOUNCE;
                    hits.pounces += 1;
                    hits.pops += 1;
                    hits.pop_xy = (bx + bw / 2.0, by);
                    hits.fx.push((pop_fx(&a.kind), bx + bw / 2.0, by)); // bopped a critter
                } else {
                    hits.hurt = true;
                }
            }
        }
    }
    hits
}

/// Draw one play frame — the camera window of tiles, the goal post, the live
/// actors, and Munchii — and present it through `backend` into `out`. Shared by
/// `run_play` and the headless `soak` crash-hunt so both exercise the same path.
#[allow(clippy::too_many_arguments)]
fn draw_play_frame(
    fb: &mut Framebuffer,
    backend: &mut dyn Backend,
    out: &mut Vec<u8>,
    world: &LevelWorld,
    sim: &Sim,
    actors: &[Actor],
    projectiles: &[Mob],
    hostiles: &[Mob],
    fb_w: usize,
    fb_h: usize,
    cols: u16,
    rows: u16,
    full_redraw: bool,
    down_held: bool,
    zoom: usize,
    shake: (f64, f64),
    hide_player: bool,
    look_ahead: f64,
) {
    let zoom = zoom.max(1);
    let pal = art::palette(world.theme);
    let pcx = sim.player.pos.x + sim.player.w / 2.0 + look_ahead;
    let pcy = sim.player.pos.y + sim.player.h / 2.0;
    let cpw = fb_w as f64 / cols.max(1) as f64; // px per terminal cell (w)
    let cph = fb_h as f64 / rows.max(1) as f64; // px per terminal cell (h)
    // The tiny-world camera (see engine `View`): it follows Munchii over a 1/zoom
    // slice of the world, which is rendered small and magnified — only the tiles
    // grow, so a constant-size Munchii is dwarfed. Pixel backends scroll per-pixel;
    // cell-sampling backends snap to the pre-magnification cell grid (no flicker).
    let mut view = scamper::level::View::centered(pcx, pcy, fb_w, fb_h, world.px_w(), world.px_h(), zoom);
    // Screen-shake nudges the camera *before* snapping, so it trembles by whole
    // pixels (smooth tiers) or whole cells (text tiers) — jolt without flicker.
    view.cam_x += shake.0;
    view.cam_y += shake.1;
    if backend.pixel_exact() {
        view.snap_pixels();
    } else {
        view.snap_cells(cpw, cph);
    }
    let (view_w, view_h) = (view.view_w, view.view_h);
    let (cam_x, cam_y) = (view.cam_x, view.cam_y);
    let sx = |wx: f64| view.sx(wx);
    let sy = |wy: f64| view.sy(wy);

    // Render the environment (tiles + goal) into a small buffer, then magnify.
    // (At zoom 1 the buffer is unused — a 1×1 placeholder keeps the binding live.)
    let mut wfb_store = Framebuffer::new(if zoom == 1 { 1 } else { view_w }, if zoom == 1 { 1 } else { view_h });
    let env: &mut Framebuffer = if zoom == 1 { fb } else { &mut wfb_store };
    env.clear(pal.sky);
    // Parallax backdrop behind the tiles (theme-specific motif; scrolls slower).
    let (ew, eh) = (env.width, env.height);
    art::draw_backdrop(env, world.theme, &pal, cam_x, ew, eh);
    let t = TILE as i32;
    let tx0 = (cam_x / TILE).floor() as i32;
    let tx1 = ((cam_x + view_w as f64) / TILE).ceil() as i32;
    let ty0 = (cam_y / TILE).floor() as i32;
    let ty1 = ((cam_y + view_h as f64) / TILE).ceil() as i32;
    for ty in ty0..ty1 {
        for tx in tx0..tx1 {
            if let Some(kind) = world.kind_at(tx, ty) {
                art::draw_tile(env, tx * t - cam_x as i32, ty * t - cam_y as i32, kind, &pal);
            }
        }
    }
    // goal post (drawn into the environment buffer so it magnifies with the tiles)
    if let Some((gx, gy)) = world.goal {
        let gsx = (gx - cam_x) as i32;
        env.fill_rect(gsx, 0, 2, view_h as i32, Rgba::rgb(235, 235, 245));
        env.fill_rect(gsx - 7, (gy - cam_y) as i32, 7, 5, Rgba::rgb(232, 84, 84));
    }
    if zoom != 1 {
        fb.upscale_from(&wfb_store, zoom);
    }
    // Build the on-screen sprites: world entities (creatures / items) first, then
    // Munchii on top. Each carries its own palette so the colored tiers draw it in
    // its own colors. A sprite = (glyph rows, top-left px, palette).
    type Drawable = (Vec<String>, f64, f64, fn(char) -> (u8, u8, u8));
    let mut sprites: Vec<Drawable> = Vec::new();

    for a in actors {
        if !a.mob.alive {
            continue;
        }
        let sp = match scamper::sprite::get(&a.kind) {
            Some(s) => s,
            None => continue, // a kind we haven't authored a sprite for yet
        };
        let exw = a.mob.pos.x + a.mob.w / 2.0; // entity center-x, world px
        if exw < cam_x - TILE || exw > cam_x + view_w as f64 + TILE {
            continue; // cull off-screen
        }
        // A curled rollo shows its "curl" frames (falls back to "walk" if none).
        let an = sp.anim(if a.mode == Mode::Walk { "walk" } else { "curl" });
        let n = an.frames.len().max(1);
        let fi = (sim.clock() / (NS_PER_SEC / an.fps.max(1) as u64)) as usize % n;
        // Creatures face their walk direction; items don't flip.
        let elines: Vec<String> = if !a.item && a.mob.facing < 0 {
            an.frames[fi].iter().map(|l| flip_line(l)).collect()
        } else {
            an.frames[fi].iter().map(|s| s.to_string()).collect()
        };
        let efw = elines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
        let (emw, emh) = (efw * cpw, elines.len() as f64 * cph);
        let elx = sx(exw) - emw / 2.0;
        let ely = sy(a.mob.pos.y + a.mob.h) - emh; // feet at the mob's bottom
        sprites.push((elines, elx, ely, sp.palette));
    }

    // Sudsball projectiles.
    if let Some(sp) = scamper::sprite::get("sudsball") {
        let an = sp.anim("fly");
        let n = an.frames.len().max(1);
        let fi = (sim.clock() / (NS_PER_SEC / an.fps.max(1) as u64)) as usize % n;
        for p in projectiles {
            let pl: Vec<String> = an.frames[fi].iter().map(|s| s.to_string()).collect();
            let pfw = pl.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
            let (pmw, pmh) = (pfw * cpw, pl.len() as f64 * cph);
            let plx = sx(p.pos.x + p.w / 2.0) - pmw / 2.0;
            let ply = sy(p.pos.y + p.h / 2.0) - pmh / 2.0;
            sprites.push((pl, plx, ply, sp.palette));
        }
    }

    // Hostile thrown sticks.
    if let Some(sp) = scamper::sprite::get("stick") {
        let an = sp.anim("fly");
        let n = an.frames.len().max(1);
        let fi = (sim.clock() / (NS_PER_SEC / an.fps.max(1) as u64)) as usize % n;
        for h in hostiles {
            let hl: Vec<String> = an.frames[fi].iter().map(|s| s.to_string()).collect();
            let hfw = hl.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
            let (hmw, hmh) = (hfw * cpw, hl.len() as f64 * cph);
            let hlx = sx(h.pos.x + h.w / 2.0) - hmw / 2.0;
            let hly = sy(h.pos.y + h.h / 2.0) - hmh / 2.0;
            sprites.push((hl, hlx, hly, sp.palette));
        }
    }

    // Munchii himself, centered on the hitbox with his feet on its bottom edge.
    // (Skipped on blink frames during post-hit invulnerability — the i-frame flicker.)
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
    let cx = sx(sim.player.pos.x + sim.player.w / 2.0);
    let bottom = sy(sim.player.pos.y + sim.player.h);
    if !hide_player {
        sprites.push((lines, cx - mw / 2.0, bottom - mh, munchii::beagle_rgb as fn(char) -> (u8, u8, u8)));
    }

    // Transient effects (puffs, bonks, pops, sparkles) — world-anchored in sim.fx.
    let fxr = sim.fx.render(sim.clock());
    let words = sim.fx.render_words(sim.clock()); // floating "BONK!"/"WOAH!" shouts

    if backend.draws_overlay() {
        // character tiers: stamp each sprite's glyphs (one per cell) over the scene
        let mut overlays: Vec<Overlay> = sprites
            .iter()
            .enumerate()
            .map(|(i, (lns, lx, ly, pal))| Overlay {
                lines: lns,
                col: (lx / cpw).round() as i32,
                row: (ly / cph).round() as i32,
                tint: None,
                palette: Some(*pal),
                z: i as i32,
            })
            .collect();
        // Effect clips (uniform tint), composited on top, camera-offset.
        let fx_lines: Vec<Vec<String>> = fxr.iter().map(|(frame, ..)| frame.iter().map(|s| s.to_string()).collect()).collect();
        for ((frame, tint, z, fxx, fxy), lns) in fxr.iter().zip(fx_lines.iter()) {
            let w = frame.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
            let ex = sx(*fxx) - w * cpw / 2.0;
            let ey = sy(*fxy);
            overlays.push(Overlay { lines: lns, col: (ex / cpw).round() as i32, row: (ey / cph).round() as i32, tint: Some(*tint), palette: None, z: 1000 + z });
        }
        // Word shouts (single text line, centered, drawn above everything).
        let word_lines: Vec<Vec<String>> = words.iter().map(|(t, ..)| vec![t.to_string()]).collect();
        for ((text, tint, z, wx, wy), lns) in words.iter().zip(word_lines.iter()) {
            let ex = sx(*wx) - text.chars().count() as f64 * cpw / 2.0;
            overlays.push(Overlay { lines: lns, col: (ex / cpw).round() as i32, row: (sy(*wy) / cph).round() as i32, tint: Some(*tint), palette: None, z: 1000 + z });
        }
        backend.present(out, fb, cols, rows, full_redraw, &overlays);
    } else {
        // pixel tiers: rasterize each sprite into the framebuffer in its colors
        for (lns, lx, ly, pal) in &sprites {
            draw_sprite_pixels(fb, lns, *lx, *ly, cpw, cph, *pal);
        }
        for &(frame, tint, _z, fxx, fxy) in &fxr {
            draw_effect_pixels(fb, frame, tint, sx(fxx), sy(fxy), cpw, cph);
        }
        for &(text, tint, _z, wx, wy) in &words {
            draw_effect_pixels(fb, &[text], tint, sx(wx) - text.chars().count() as f64 * cpw / 2.0, sy(wy), cpw, cph);
        }
        backend.present(out, fb, cols, rows, full_redraw, &[]);
    }
}

/// `supermunchii soak [dir]` — headless crash-hunt: load every `*.lvl` under `dir`
/// (default `imported/lvl`) and run each through the sim + render pipeline for a
/// few hundred ticks (walking right, jumping), catching panics per level. No
/// terminal needed; panic details land in `scamp.log` (run with `--debug`).
fn run_soak(dir: &str) {
    let mut files = Vec::new();
    let path = std::path::Path::new(dir);
    if path.is_file() {
        files.push(dir.to_string()); // soak a single .lvl directly
    } else {
        collect_lvls(path, &mut files);
        files.sort();
    }
    if files.is_empty() {
        eprintln!("soak: no .lvl files under {dir}");
        std::process::exit(2);
    }
    let mut ok = 0usize;
    let mut fails: Vec<String> = Vec::new();
    let mut stalls: Vec<String> = Vec::new();
    for path in &files {
        let p = path.clone();
        // 1500 ticks (~25s) — long enough to run-and-jump across a full ~200-tile
        // level, so a low reach really means "stuck", not "ran out of time".
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| soak_level(&p, 1500))) {
            Ok(Ok(s)) => {
                ok += 1;
                let pct = (s.reached / s.width * 100.0).round() as i32;
                if s.stalled {
                    stalls.push(format!("{path}: stuck at {pct}% (x={:.0} of {:.0})", s.reached, s.width));
                }
            }
            Ok(Err(e)) => fails.push(format!("FAIL  {path}: {e}")),
            Err(_) => fails.push(format!("PANIC {path}  (message + backtrace in scamp.log if --debug)")),
        }
    }
    eprintln!("soak: {ok}/{} ran clean (no crash)", files.len());
    for f in &fails {
        eprintln!("  {f}");
    }
    if !stalls.is_empty() {
        eprintln!("\nstalled — run-and-jump couldn't get through ({}):", stalls.len());
        for s in &stalls {
            eprintln!("  {s}");
        }
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

/// `supermunchii slice [width]` — cut every imported level into small
/// de-identified slices and write the committed slice database (the source levels
/// can't ship, but tiny recombinable chunks can). Dev-only; needs the source.
fn run_slice(args: &[String]) {
    let width: i32 = nth_nonflag(args, 1).and_then(|s| s.parse().ok()).unwrap_or(8);
    let mut files = Vec::new();
    collect_lvls(std::path::Path::new("imported/lvl"), &mut files);
    files.retain(|f| !f.ends_with("megalevel.lvl"));
    files.sort();
    if files.is_empty() {
        eprintln!("slice: no levels under imported/lvl — run `supermunchii import` first");
        std::process::exit(2);
    }
    let mut seen = std::collections::HashSet::new();
    let mut slices: Vec<Level> = Vec::new();
    for f in &files {
        if let Ok(lvl) = load_level_file(f) {
            for s in scamper::level::slice_level(&lvl, width) {
                if s.tiles.is_empty() && s.entities.is_empty() {
                    continue; // skip empty (all-sky) chunks
                }
                if seen.insert(scamper::level::slice_fingerprint(&s)) {
                    slices.push(s); // keep one of each distinct chunk
                }
            }
        }
    }
    // Bound the DB to a varied sample (smaller repo footprint, and retaining less
    // of any source). Seeded shuffle → take CAP.
    const CAP: usize = 1200;
    let total = slices.len();
    if total > CAP {
        let mut state = 0x5DEE_CE66u64;
        for i in (1..slices.len()).rev() {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            slices.swap(i, (state >> 33) as usize % (i + 1));
        }
        slices.truncate(CAP);
    }
    let out = format!("{}/slices.pack", env!("CARGO_MANIFEST_DIR"));
    if let Err(e) = std::fs::write(&out, scamper::level::pack_levels(&slices)) {
        eprintln!("slice: cannot write {out}: {e}");
        std::process::exit(2);
    }
    eprintln!("slice: {} of {total} distinct slices (width {width}) -> {out}", slices.len());
}

/// The committed, de-identified slice database, embedded in the binary so the
/// random-walk works for anyone — no (un-shippable) source levels required.
/// Regenerate with `supermunchii slice`.
const SLICE_DB: &str = include_str!("../slices.pack");

/// Build a random test level of `count` segments (seeded). Random-walks the
/// embedded slice DB if present; otherwise falls back to stitching whole imported
/// levels (only available where the source has been imported locally).
fn build_mega(count: usize, seed: u64) -> Option<Level> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    let mut rng = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };

    // Preferred: a random walk over the slice DB (sampling with replacement).
    let slices = scamper::level::parse_pack(SLICE_DB);
    if !slices.is_empty() {
        use scamper::level::slice::SLICE_H;
        use scamper::level::{TileKind, TileSpan};
        // A flat runway first, so the romp always starts walkable (a random first
        // slice could otherwise wall Munchii in right at spawn).
        let mut runway = Level::new("runway", "overworld", 8, SLICE_H);
        runway.spawn = (2, SLICE_H - 3);
        runway.tiles.push(TileSpan { x: 0, y: SLICE_H - 1, len: 8, kind: TileKind::Ground });
        runway.tiles.push(TileSpan { x: 0, y: SLICE_H - 2, len: 8, kind: TileKind::Ground });
        let mut parts = vec![runway];
        parts.extend((0..count.max(1)).map(|_| slices[rng() % slices.len()].clone()));
        return Some(scamper::level::stitch(&parts, 3));
    }

    // Fallback: stitch whole imported levels (dev machines with the source).
    let mut files = Vec::new();
    collect_lvls(std::path::Path::new("imported/lvl"), &mut files);
    files.retain(|f| !f.ends_with("megalevel.lvl"));
    files.sort();
    if files.is_empty() {
        return None;
    }
    for i in (1..files.len()).rev() {
        files.swap(i, rng() % (i + 1));
    }
    let picked: Vec<Level> = files.iter().take(count.min(24).max(1)).filter_map(|f| load_level_file(f).ok()).collect();
    Some(scamper::level::stitch(&picked, 6))
}

/// `supermunchii mega [out.lvl] [count] [seed]` — stitch a large random sampling
/// of the imported levels into one giant goal-less test level (a red-team romp
/// through every system; also our own remix, no longer any original's layout).
fn run_mega(args: &[String]) {
    let out = nth_nonflag(args, 1).unwrap_or("imported/lvl/megalevel.lvl").to_string();
    let count: usize = nth_nonflag(args, 2).and_then(|s| s.parse().ok()).unwrap_or(80);
    let seed: u64 = nth_nonflag(args, 3).and_then(|s| s.parse().ok()).unwrap_or(1);

    let Some(mega) = build_mega(count, seed) else {
        eprintln!("mega: no levels under imported/lvl — run `supermunchii import` first");
        std::process::exit(2);
    };
    if let Err(e) = std::fs::write(&out, mega.to_text()) {
        eprintln!("mega: cannot write {out}: {e}");
        std::process::exit(2);
    }
    eprintln!(
        "mega: stitched (seed {seed}) -> {out}: {}x{} tiles, {} spans, {} entities",
        mega.w, mega.h, mega.tiles.len(), mega.entities.len()
    );
    eprintln!("  play it:  ./run.sh play {out}");
}

/// The default play target: a fresh random megalevel (re-stitched each launch via
/// a wall-clock seed) written to `imported/lvl/megalevel.lvl`. Falls back to the
/// shipped authored level when nothing has been imported.
fn default_test_level() -> String {
    let authored = format!("{}/levels/yard-romp-1.lvl", env!("CARGO_MANIFEST_DIR"));
    match build_mega(80, now_ns()) {
        Some(mega) => {
            let out = "imported/lvl/megalevel.lvl";
            match std::fs::write(out, mega.to_text()) {
                Ok(_) => {
                    eprintln!("play: fresh random megalevel ({}x{}, {} entities)", mega.w, mega.h, mega.entities.len());
                    out.to_string()
                }
                Err(_) => authored,
            }
        }
        None => authored,
    }
}

/// Run one level headlessly for `ticks` ticks through the same sim + render as
/// `run_play`, **holding right and repeatedly jumping** — the input that carries
/// Munchii through most of any level. Renders periodically through all four
/// backends so backend-specific draw crashes surface too. Returns `Err` on a
/// clean failure; a panic propagates (the soak/test catches it per level).
/// How a level fared under the run-and-jump soak.
struct SoakStats {
    reached: f64, // furthest x (px) Munchii got
    width: f64,   // level width (px)
    stalled: bool, // made no forward progress for a long stretch, short of the end
}

fn soak_level(path: &str, ticks: u64) -> Result<SoakStats, String> {
    let level = load_level_file(path).map_err(|e| format!("load: {e}"))?;
    let world = LevelWorld::from_level(&level);
    let mut sim = sim_at(world.spawn);
    let mut actors = build_actors(&world);
    let mut hostiles: Vec<Mob> = Vec::new();
    let mut kibble = 0u32;
    let mut power = Power::Small;
    let ws = terminal::WinSize { cols: 80, rows: 24, xpix: 640, ypix: 384 };
    let (fb_w, fb_h, cols, rows) = play_view(ws);
    let mut fb = Framebuffer::new(fb_w, fb_h);
    let mut backends: [Box<dyn Backend>; 4] =
        [Box::new(KittyBackend::new()), Box::new(TextBackend::new()), Box::new(AsciiBackend::new()), Box::new(MonoBackend::new())];
    let mut out: Vec<u8> = Vec::new();

    // Progress tracking: furthest x reached, and how long since it last advanced.
    const STALL_TICKS: u64 = 420; // ~7s of no forward progress = stuck
    let mut reached = sim.player.pos.x;
    let mut last_gain = 0u64;

    for tick in 0..ticks {
        // Hold right; tap jump on a ~18-tick cadence (held ~10) to clear gaps.
        let inp = InputFrame { axis_x: 1, jump_pressed: tick % 18 == 0, jump_held: tick % 18 < 10, down_held: false };
        sim.step(&world.map, inp);
        let _ = step_actors(&mut actors, &world.map, &mut sim.player, &mut kibble, &mut power, false);
        emit_thrower_sticks(&actors, &mut hostiles, &sim.player);
        let _ = step_hostiles(&mut hostiles, &world.map, &sim.player);
        let (px, py, pw, ph) = (sim.player.pos.x, sim.player.pos.y, sim.player.w, sim.player.h);
        if px > reached + 1.0 {
            reached = px;
            last_gain = tick;
        }
        if world.hazard_overlap(px, py, pw, ph) {
            sim = sim_at(world.spawn);
        }
        if tick % 15 == 0 {
            // Alternate zoom so the soak crash-tests both the magnified (tiny) and
            // 1× render paths headlessly.
            let z = if (tick / 15) % 2 == 0 { 4 } else { 1 };
            for b in backends.iter_mut() {
                draw_play_frame(&mut fb, b.as_mut(), &mut out, &world, &sim, &actors, &[], &hostiles, fb_w, fb_h, cols, rows, true, false, z, (0.0, 0.0), false, 0.0);
            }
            // Also render the status line like run_play does — at narrow widths too,
            // so status formatting (multibyte truncation, etc.) is exercised, not
            // just the scene. (This is the gap that hid the is_char_boundary crash.)
            let mut status = String::new();
            for w in [10u16, 28, 48, cols] {
                let won = tick > ticks * 3 / 4; // exercise the LEVEL COMPLETE banner too
                render_play_status(&mut status, &level, sim.player.state, "mono", won, false, kibble, 3, power, false, false, false, 0, 1, w);
            }
        }
    }
    let width = world.px_w().max(1.0);
    // Stuck = stopped advancing well before the end (not just "ran out of ticks").
    let stalled = (ticks.saturating_sub(last_gain) > STALL_TICKS) && reached < width - 6.0 * TILE;
    Ok(SoakStats { reached, width, stalled })
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn render_play_status(buf: &mut String, level: &Level, st: State, backend: &str, won: bool, game_over: bool, kibble: u32, lives: i32, power: Power, boost: bool, glide: bool, inv: bool, secs: u32, rows: u16, cols: u16) {
    use std::fmt::Write;
    let mut plain = String::new();
    let zoom = if boost { "  ⚡zoom" } else { "" }; // active Zoomies-Treat burst
    let wings = if glide { "  ~glide" } else { "" }; // Flutter Collar unlocked
    let star = if inv { "  ★star" } else { "" }; // invincibility burst
    let clock = format!("{}:{:02}", secs / 60, secs % 60); // level timer
    if game_over {
        let _ = write!(plain, "✗ GAME OVER ✗   kibble:{kibble}   gfx:{backend} · q quit");
    } else if won {
        let _ = write!(plain, "★ LEVEL COMPLETE — {} ★   ♥×{lives} kibble:{kibble}  time {clock}   → next level…   gfx:{backend} · q quit", level.id);
    } else {
        let _ = write!(plain, "{}  [{}]  {}  ♥×{lives}  kibble {kibble} ({}/100→1up)  gear:{}{zoom}{wings}{star}  t{clock}   h help · Tab gfx:{backend} · q quit", level.id, level.theme, state_letter(st), kibble % 100, power.label());
    }
    let maxw = (cols as usize).saturating_sub(1);
    if plain.chars().count() > maxw {
        plain = plain.chars().take(maxw).collect(); // clamp by chars: multibyte-safe
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
        palette: None,
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
fn draw_sprite_pixels(fb: &mut Framebuffer, lines: &[String], lx: f64, ly: f64, cpw: f64, cph: f64, palette: fn(char) -> (u8, u8, u8)) {
    for (gr, line) in lines.iter().enumerate() {
        for (gc, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let (r, g, b) = palette(ch);
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
        overlays.push(Overlay { lines: &lines, col: pcol, row: prow, tint: None, palette: None, z: 0 });
        for (fl, tint, z, col, row) in &fxr {
            overlays.push(Overlay { lines: fl, col: *col, row: *row, tint: Some(*tint), palette: None, z: *z });
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
        draw_sprite_pixels(fb, &lines, pcol as f64 * cpw, prow as f64 * cph, cpw, cph, munchii::beagle_rgb);
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
    overlays.push(Overlay { lines: &lines, col: pcol, row: prow, tint: None, palette: None, z: 0 });
    for (fl, tint, z, col, row) in &fxr {
        overlays.push(Overlay { lines: fl, col: *col, row: *row, tint: Some(*tint), palette: None, z: *z });
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
    if plain.chars().count() > maxw {
        plain = plain.chars().take(maxw).collect(); // clamp by chars: multibyte-safe
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

/// The opening title card. Any key (or a ~4s timeout, so unattended runs aren't
/// blocked) starts the game; q/Esc quits — returns false then.
fn show_title_card(out: &mut Vec<u8>, input: &mut Input, cols: u16, rows: u16) -> bool {
    use std::io::Write;
    out.clear();
    out.extend_from_slice(b"\x1b[2J");
    let cen = |out: &mut Vec<u8>, row: i32, s: &str| {
        let col = ((cols as i32 - s.chars().count() as i32) / 2).max(0) + 1;
        let _ = write!(out, "\x1b[{};{}H{}", row.max(1), col, s);
    };
    let mid = (rows as i32 / 2).max(2);
    cen(out, mid - 2, "\x1b[1m★  S U P E R   M U N C H I I  ★\x1b[0m");
    cen(out, mid, "a sample game on the scamper engine");
    cen(out, mid + 3, "\x1b[7m press any key to start \x1b[0m");
    cen(out, mid + 4, "move: arrows · jump: \u{2191} · throw: space · h help · q quit");
    {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(out);
        let _ = o.flush();
    }
    let deadline = now_ns() + 4_000_000_000;
    loop {
        if terminal::quit_requested() {
            return false;
        }
        input.poll();
        if input.quit || input.pressed(K_Q) || input.pressed(K_ESC) {
            return false;
        }
        if input.any_pressed() || now_ns() >= deadline {
            return true;
        }
        sleep_until_ns(now_ns() + 16_000_000, 1_000_000);
    }
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

/// Level-play controls overlay (toggled with `h`). Plain adaptive text lines, so
/// it reads at any terminal size — the full control list the status bar can't fit.
fn render_play_help(out: &mut Vec<u8>, active_backend: &str) {
    out.clear();
    let mut r = 2u16;
    hline(out, r, "\x1b[1mSUPER MUNCHII — controls\x1b[0m");
    r += 2;
    hline(out, r, "Move / run        A / D   or   \u{2190} / \u{2192}   (hold to build speed)");
    r += 1;
    hline(out, r, "Jump (hold=higher)  Z / K / W / \u{2191}   (\u{2191} is jump — Space is throw)");
    r += 1;
    hline(out, r, "Double jump       jump again in mid-air");
    r += 1;
    hline(out, r, "Crouch / pipe     S / \u{2193}   (enter a pipe while standing on it)");
    r += 1;
    hline(out, r, "Throw Sudsball    Space (or C)  \u{2022}  always ready — bonks critters");
    r += 1;
    hline(out, r, "Dash (dodge)      X   \u{2022}  a quick burst with brief invulnerability");
    r += 1;
    hline(out, r, "Assist (practice) G   \u{2022}  toggle invulnerability to learn a level");
    r += 2;
    hline(out, r, "\x1b[1mPower-ups (gear, not damage)\x1b[0m");
    r += 1;
    hline(out, r, "  Big Kibble      small \u{2192} big   (tougher; snappier throw)");
    r += 1;
    hline(out, r, "  Bubble Bone     big \u{2192} bubble   (fast, far Sudsballs)");
    r += 1;
    hline(out, r, "  Lucky Squeaky / 100 kibble = an extra life");
    r += 1;
    hline(out, r, "  A hit drops one gear tier; a hit while small = a life");
    r += 2;
    hline(out, r, "Bonk blocks from below  bricks shatter, ? blocks give a treat");
    r += 1;
    hline(out, r, "Pounce critters from above  \u{2022}  reach the bath plug to finish");
    r += 2;
    hline(out, r, &format!("Tab  switch graphics [now: {active_backend}]   \u{2022}   p  pause   \u{2022}   h  close   \u{2022}   Q  quit"));
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
    if plain.chars().count() > maxw {
        plain = plain.chars().take(maxw).collect(); // clamp by chars: multibyte-safe
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
    use scamper::level::ir::{Entity, TileKind, TileSpan};

    /// The level view + scene render + status must adapt to any window size and
    /// aspect ratio (tiny, ultrawide, tall-narrow) without degenerate dims or panic.
    #[test]
    fn rendering_adapts_to_any_window() {
        let mut l = Level::new("t", "overworld", 30, 12);
        l.tiles.push(TileSpan { x: 0, y: 10, len: 30, kind: TileKind::Ground });
        l.spawn = (3, 8);
        let world = LevelWorld::from_level(&l);
        let mut sim = sim_at(world.spawn);
        let actors = build_actors(&world);
        let mut backend: Box<dyn Backend> = Box::new(MonoBackend::new());
        let mut out = Vec::new();
        let mut status = String::new();
        for &(cols, rows) in &[(1u16, 1u16), (20, 8), (200, 20), (40, 60), (80, 24), (320, 100), (8, 50)] {
            let ws = terminal::WinSize { cols, rows, xpix: cols.saturating_mul(8), ypix: rows.saturating_mul(16) };
            let (fb_w, fb_h, vc, vr) = play_view(ws);
            assert!(fb_w > 0 && fb_h > 0 && vc > 0 && vr > 0, "play_view degenerate at {cols}x{rows}");
            let mut fb = Framebuffer::new(fb_w, fb_h);
            sim.step(&world.map, InputFrame { axis_x: 1, jump_pressed: false, jump_held: false, down_held: false });
            draw_play_frame(&mut fb, backend.as_mut(), &mut out, &world, &sim, &actors, &[], &[], fb_w, fb_h, vc, vr, true, false, 4, (0.0, 0.0), false, 0.0);
            render_play_status(&mut status, &l, sim.player.state, "mono", false, false, 0, 3, Power::Small, false, false, false, 0, vr + 1, vc);
        }
    }

    /// The authored campaign level's question blocks must respond to a head-bonk
    /// (release an item). Loads the real shipped level and bonks one.
    #[test]
    fn campaign_question_block_releases_on_bonk() {
        let path = format!("{}/levels/yard-romp-1.lvl", env!("CARGO_MANIFEST_DIR"));
        let level = load_level_file(&path).expect("load authored level");
        let mut world = LevelWorld::from_level(&level);
        assert!(world.map.is_solid(6, 9), "question block at (6,9) should be solid");
        let mut sim = sim_at(world.spawn);
        sim.player.pos = Vec2::new(6.0 * 16.0, 10.6 * 16.0); // just under the block (its bottom = 160)
        sim.player.vel = Vec2::new(0.0, -340.0);
        let mut got = Bonk::Nothing;
        for _ in 0..40 {
            sim.step(&world.map, InputFrame { axis_x: 0, jump_pressed: false, jump_held: false, down_held: false });
            if sim.player.bonked_head {
                let cx = ((sim.player.pos.x + sim.player.w / 2.0) / TILE).floor() as i32;
                let cy = ((sim.player.pos.y - 1.0) / TILE).floor() as i32;
                let b = world.bonk(cx, cy);
                if b != Bonk::Nothing {
                    got = b;
                    break;
                }
            }
        }
        assert!(matches!(got, Bonk::Released(_)), "bonking the question should release an item, got {got:?}");
    }

    /// The tiny-world zoom must actually magnify the environment: at 4× the ground
    /// occupies a much thicker on-screen band than at 1× (a single tile blows up
    /// to a 4-row block), while Munchii's sprite stays a constant glyph size.
    #[test]
    fn zoom_magnifies_the_environment() {
        use std::cell::RefCell;
        use std::rc::Rc;
        struct Cap(Rc<RefCell<String>>);
        impl Backend for Cap {
            fn name(&self) -> &'static str {
                "cap"
            }
            fn draws_overlay(&self) -> bool {
                true
            }
            fn present(&mut self, _o: &mut Vec<u8>, fb: &Framebuffer, cols: u16, rows: u16, _f: bool, ov: &[Overlay]) {
                *self.0.borrow_mut() = scamper::backend::mono_text(fb, cols as usize, rows as usize, ov);
            }
            fn teardown(&mut self, _o: &mut Vec<u8>) {}
        }
        let path = format!("{}/levels/yard-romp-1.lvl", env!("CARGO_MANIFEST_DIR"));
        let level = load_level_file(&path).expect("load authored level");
        let world = LevelWorld::from_level(&level);
        let (cols, rows) = (60u16, 18u16);
        let (fb_w, fb_h) = (cols as usize * 8, rows as usize * 16);
        // Count bottom screen rows that are "ground-dense" (mostly non-blank).
        let ground_band = |grid: &str| -> usize {
            grid.lines().rev().take_while(|l| l.chars().filter(|c| *c != ' ').count() > cols as usize / 2).count()
        };
        let band_at = |zoom: usize| -> usize {
            let mut sim = sim_at(world.spawn);
            resize_player(&mut sim.player, BODY_W / zoom as f64, BODY_H / zoom as f64);
            for _ in 0..30 {
                sim.step(&world.map, InputFrame { axis_x: 1, jump_pressed: false, jump_held: false, down_held: false });
            }
            let cap = Rc::new(RefCell::new(String::new()));
            let mut be: Box<dyn Backend> = Box::new(Cap(cap.clone()));
            let mut fb = Framebuffer::new(fb_w, fb_h);
            let actors = build_actors(&world);
            draw_play_frame(&mut fb, be.as_mut(), &mut Vec::new(), &world, &sim, &actors, &[], &[], fb_w, fb_h, cols, rows, true, false, zoom, (0.0, 0.0), false, 0.0);
            let b = ground_band(&cap.borrow());
            b
        };
        let (one, four) = (band_at(1), band_at(4));
        assert!(four >= one * 2, "4× zoom should thicken the ground band (1×={one}, 4×={four})");
    }

    /// Replicate run_play's head-bonk path: a brick directly above Munchii must
    /// shatter when he jumps into it.
    #[test]
    fn head_bonk_shatters_a_brick() {
        let mut l = Level::new("t", "overworld", 12, 12);
        l.tiles.push(TileSpan { x: 0, y: 8, len: 12, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "brick".into(), x: 3, y: 5, props: vec![] });
        l.spawn = (3, 7);
        let mut world = LevelWorld::from_level(&l);
        assert!(world.map.is_solid(3, 5), "brick starts solid");
        let mut sim = sim_at(world.spawn);
        // Place Munchii directly under the brick (brick bottom = 6*16 = 96) and launch up.
        sim.player.pos = Vec2::new(3.0 * 16.0, 6.5 * 16.0);
        sim.player.vel = Vec2::new(0.0, -320.0);

        let mut result = Bonk::Nothing;
        for _ in 0..40 {
            sim.step(&world.map, InputFrame { axis_x: 0, jump_pressed: false, jump_held: false, down_held: false });
            if sim.player.bonked_head {
                let cx = ((sim.player.pos.x + sim.player.w / 2.0) / TILE).floor() as i32;
                let cy = ((sim.player.pos.y - 1.0) / TILE).floor() as i32;
                let b = world.bonk(cx, cy);
                if b != Bonk::Nothing {
                    result = b;
                    break;
                }
            }
        }
        assert_eq!(result, Bonk::Broke(None), "head-bonk should shatter the brick, got {result:?}");
        assert!(!world.map.is_solid(3, 5), "broken brick should no longer be solid");
    }

    /// A spiky Prickle can't be pounced — landing on it hurts, and it survives —
    /// but a Sudsball pops it. This is the whole point of the throw.
    #[test]
    fn prickle_hurts_on_pounce_but_pops_to_a_sudsball() {
        let mut l = Level::new("t", "overworld", 16, 10);
        l.tiles.push(TileSpan { x: 0, y: 8, len: 16, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "prickle".into(), x: 6, y: 7, props: vec![] });
        let world = LevelWorld::from_level(&l);
        let mut actors = build_actors(&world);
        let prickle = actors.iter().position(|a| a.kind == "prickle").expect("prickle built");
        let (bx, by, bw, _bh) = (actors[prickle].mob.pos.x, actors[prickle].mob.pos.y, actors[prickle].mob.w, actors[prickle].mob.h);

        // Munchii drops squarely onto its crown (a "pounce"): it must hurt, not pop.
        let mut player = Player::new(bx, by - 8.0); // overlapping from above
        player.vel.y = 240.0; // falling
        let mut kibble = 0;
        let mut power = Power::Big; // big so a hit just drops a tier (no respawn needed)
        let hits = step_actors(&mut actors, &world.map, &mut player, &mut kibble, &mut power, false);
        assert!(hits.hurt, "pouncing a prickle hurts");
        assert!(actors[prickle].mob.alive, "and the prickle survives the pounce");

        // A Sudsball on it pops it (into kibble).
        let mut projectiles = vec![Mob::new(bx + bw / 2.0, by, 4.0, 4.0, 1, 0.0, Gait::Fly)];
        step_projectiles(&mut projectiles, &mut actors, &world.map, &mut kibble);
        assert!(!actors[prickle].mob.alive, "a Sudsball pops the prickle");
    }

    /// Collecting a Zoomies Treat flags the speed burst (and still banks a kibble).
    #[test]
    fn zoomies_treat_triggers_the_speed_burst() {
        let mut l = Level::new("t", "overworld", 12, 10);
        l.tiles.push(TileSpan { x: 0, y: 8, len: 12, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "zoomies_treat".into(), x: 5, y: 7, props: vec![] });
        let world = LevelWorld::from_level(&l);
        let mut actors = build_actors(&world);
        let (bx, by) = (actors[0].mob.pos.x, actors[0].mob.pos.y);
        let mut player = Player::new(bx, by);
        let mut kibble = 0;
        let mut power = Power::Small;
        let hits = step_actors(&mut actors, &world.map, &mut player, &mut kibble, &mut power, false);
        assert!(hits.zoomies, "treat triggers the burst");
        assert_eq!(kibble, 1, "and banks a kibble");
        assert!(!actors[0].mob.alive, "treat is consumed");
    }

    /// A Chaser charges toward Munchii when he's in range, and idles otherwise.
    #[test]
    fn chaser_charges_toward_the_player_in_range() {
        let mut l = Level::new("t", "overworld", 30, 10);
        l.tiles.push(TileSpan { x: 0, y: 8, len: 30, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "chaser".into(), x: 15, y: 7, props: vec![] });
        let world = LevelWorld::from_level(&l);
        let mut actors = build_actors(&world);
        let cx = actors[0].mob.pos.x;
        let (mut kibble, mut power) = (0, Power::Big);

        // Player to the LEFT and in range → it should face left and charge fast.
        let mut player = Player::new(cx - 60.0, actors[0].mob.pos.y);
        let _ = step_actors(&mut actors, &world.map, &mut player, &mut kibble, &mut power, false);
        assert_eq!(actors[0].mob.facing, -1, "faces the player");
        assert!(actors[0].mob.speed > CHASE_IDLE, "charges (faster than idle)");

        // Player far away → idle patrol speed.
        let mut far = Player::new(cx + 600.0, actors[0].mob.pos.y);
        let _ = step_actors(&mut actors, &world.map, &mut far, &mut kibble, &mut power, false);
        assert!((actors[0].mob.speed - CHASE_IDLE).abs() < 1e-9, "idles when out of range");
    }

    /// Pouncing a normal critter reports a pounce (which drives the air combo).
    #[test]
    fn pouncing_reports_a_pounce() {
        let mut l = Level::new("t", "overworld", 12, 10);
        l.tiles.push(TileSpan { x: 0, y: 8, len: 12, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "boneling".into(), x: 5, y: 7, props: vec![] });
        let world = LevelWorld::from_level(&l);
        let mut actors = build_actors(&world);
        let (bx, by) = (actors[0].mob.pos.x, actors[0].mob.pos.y);
        let mut player = Player::new(bx, by - 8.0); // overlapping from above
        player.vel.y = 240.0; // falling → a stomp
        let (mut kibble, mut power) = (0, Power::Big);
        let hits = step_actors(&mut actors, &world.map, &mut player, &mut kibble, &mut power, false);
        assert_eq!(hits.pounces, 1, "a stomp counts as one pounce");
        assert!(!actors[0].mob.alive, "and pops the critter");
    }

    /// Pouncing the boss's head registers a boss hit (and bounces you), while a
    /// side touch hurts. The hit count / i-frames live in run_play.
    #[test]
    fn boss_takes_a_pounce_to_the_head() {
        let mut l = Level::new("t", "castle", 14, 12);
        l.tiles.push(TileSpan { x: 0, y: 9, len: 14, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "baron_whiskers".into(), x: 6, y: 8, props: vec![] });
        let world = LevelWorld::from_level(&l);
        let mut actors = build_actors(&world);
        let i = actors.iter().position(|a| a.kind == "baron_whiskers").unwrap();
        let (bx, by, bw) = (actors[i].mob.pos.x, actors[i].mob.pos.y, actors[i].mob.w);
        let (mut kibble, mut power) = (0, Power::Big);

        // Pounce from above (falling onto his head).
        let mut p = Player::new(bx + bw / 2.0 - 6.0, by - 12.0);
        p.vel.y = 260.0;
        let hits = step_actors(&mut actors, &world.map, &mut p, &mut kibble, &mut power, false);
        assert!(hits.boss_hit, "a head pounce damages the boss");
        assert!(p.vel.y < 0.0, "and bounces Munchii back up");

        // A side touch (level with him, not descending onto the head) hurts instead.
        let mut q = Player::new(bx - 10.0, by);
        q.vel.y = 0.0;
        let hq = step_actors(&mut actors, &world.map, &mut q, &mut kibble, &mut power, false);
        assert!(!hq.boss_hit && hq.hurt, "a side bump hurts, not a hit");
    }

    /// Landing on a trampoline launches Munchii upward (flags `bounced`).
    #[test]
    fn trampoline_launches_on_landing() {
        let mut l = Level::new("t", "overworld", 12, 10);
        l.tiles.push(TileSpan { x: 0, y: 8, len: 12, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "trampoline".into(), x: 5, y: 7, props: vec![] });
        let world = LevelWorld::from_level(&l);
        let mut actors = build_actors(&world);
        let (bx, by) = (actors[0].mob.pos.x, actors[0].mob.pos.y);
        let mut player = Player::new(bx, by - 14.0); // feet just into the pad top
        player.vel.y = 300.0; // falling onto the pad
        let (mut kibble, mut power) = (0, Power::Small);
        let hits = step_actors(&mut actors, &world.map, &mut player, &mut kibble, &mut power, false);
        assert!(hits.bounced, "trampoline reports a bounce");
        assert!(player.vel.y < -300.0, "and launches Munchii sky-high (vy={})", player.vel.y);
        assert!(actors[0].mob.alive, "the pad isn't consumed");
    }

    /// While invincible, touching a critter (even side-on) bulldozes it instead of
    /// hurting Munchii — including a normally-unpouncable spiky prickle.
    #[test]
    fn star_bulldozes_critters_without_harm() {
        let mut l = Level::new("t", "overworld", 12, 10);
        l.tiles.push(TileSpan { x: 0, y: 8, len: 12, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "prickle".into(), x: 5, y: 7, props: vec![] });
        let world = LevelWorld::from_level(&l);
        let mut actors = build_actors(&world);
        let (bx, by) = (actors[0].mob.pos.x, actors[0].mob.pos.y);
        let mut player = Player::new(bx, by); // level side-on overlap (would normally hurt)
        let (mut kibble, mut power) = (0, Power::Small);
        let hits = step_actors(&mut actors, &world.map, &mut player, &mut kibble, &mut power, true);
        assert!(!hits.hurt, "invincible: no harm");
        assert!(!actors[0].mob.alive, "the spiky prickle is bulldozed");
        assert_eq!(kibble, 2, "and pays out");
    }

    /// Collecting a Flutter Collar unlocks gliding (flags `collar`).
    #[test]
    fn flutter_collar_unlocks_glide() {
        let mut l = Level::new("t", "overworld", 12, 10);
        l.tiles.push(TileSpan { x: 0, y: 8, len: 12, kind: TileKind::Ground });
        l.entities.push(Entity { kind: "flutter_collar".into(), x: 5, y: 7, props: vec![] });
        let world = LevelWorld::from_level(&l);
        let mut actors = build_actors(&world);
        let (bx, by) = (actors[0].mob.pos.x, actors[0].mob.pos.y);
        let mut player = Player::new(bx, by);
        let (mut kibble, mut power) = (0, Power::Small);
        let hits = step_actors(&mut actors, &world.map, &mut player, &mut kibble, &mut power, false);
        assert!(hits.collar, "collar unlocks gliding");
        assert!(!actors[0].mob.alive, "collar is consumed");
    }

    /// Soak each given level by holding right and jumping; collect any that panic
    /// or error so the assertion names the offenders rather than dying on the first.
    fn soak_all(files: &[String], ticks: u64) -> Vec<String> {
        let mut fails = Vec::new();
        for path in files {
            let p = path.clone();
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| soak_level(&p, ticks))) {
                Ok(Ok(_)) => {}
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

    /// Status lines hold multibyte glyphs (♥ ★ ✗ · → ↓), so clamping them to the
    /// terminal width must count characters, not bytes — a byte split panics
    /// (`is_char_boundary`). The soak renders the *scene* but never the status
    /// line, so this slipped through and crashed real play / level-complete.
    /// Hammer every status renderer at every width and mode here.
    #[test]
    fn status_lines_never_split_a_glyph_at_any_width() {
        let lvl = Level::new("yard-1-1", "overworld", 40, 12);
        let p = Player::new(32.0, 32.0);
        let mut buf = String::new();
        for cols in 0..=200u16 {
            let rows = cols.saturating_add(1);
            for &won in &[false, true] {
                for &over in &[false, true] {
                    render_play_status(&mut buf, &lvl, State::Grounded, "mono", won, over, 12_345, 3, Power::Bubble, true, true, true, 99, rows, cols);
                }
            }
            render_tiles_status(&mut buf, Theme::Castle, "ascii", rows, cols);
            render_replay_status(&mut buf, "run-1", 30, 120, "text", rows, cols, true);
            render_status(&mut buf, &p, 999, 60.0, "kitty", rows, cols, true);
        }
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

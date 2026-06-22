//! `scamp` — the game binary: a sandbox platformer level driven by keyboard,
//! rendered to a Kitty terminal. Also a headless `verify` mode that runs scripted
//! scenarios and dumps PNGs (for development on a box without a Kitty terminal).

use scamper::framebuffer::{Framebuffer, Rgba};
use scamper::input::Input;
use scamper::math::Vec2;
use scamper::player::{FeelParams, Player, State};
use scamper::time::{now_ns, sleep_until_ns, NS_PER_SEC};
use scamper::world::{TileMap, TILE};
use scamper::{kitty, terminal};
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("verify") => {
            let dir = args.get(2).map(|s| s.as_str()).unwrap_or(".");
            run_verify(dir);
        }
        Some("info") => {
            let ws = terminal::query_winsize();
            println!("winsize: {ws:?}");
        }
        _ => run_live(),
    }
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

/// Render the whole map (no camera) plus the player at `rpos` (interpolated).
fn render(fb: &mut Framebuffer, map: &TileMap, rpos: Vec2, player: &Player) {
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
    // player
    let px = rpos.x.round() as i32;
    let py = rpos.y.round() as i32;
    let pw = player.w as i32;
    let ph = player.h as i32;
    let col = state_color(player.state);
    fb.fill_rect(px, py, pw, ph, col);
    fb.stroke_rect(px, py, pw, ph, Rgba::rgb(255, 245, 210));
    // facing "eye"
    let eye_x = if player.facing >= 0 { px + pw - 4 } else { px + 1 };
    fb.fill_rect(eye_x, py + 4, 3, 3, Rgba::rgb(20, 20, 20));
    // velocity vector (debug overlay)
    let cx = px + pw / 2;
    let cy = py + ph / 2;
    let vscale = 0.06;
    fb.line(
        cx,
        cy,
        cx + (player.vel.x * vscale) as i32,
        cy + (player.vel.y * vscale) as i32,
        Rgba::rgb(255, 80, 80),
    );
}

// ---------------------------------------------------------------------------
// Live loop
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
    if !kitty_kbd {
        // Not fatal, but variable-jump / clean release won't work well.
        // (Message shown after teardown via the guard would be cleaner; keep simple.)
    }

    let map = build_sandbox();
    let fp = FeelParams::default();
    let mut player = Player::new(map.spawn.0, map.spawn.1);
    let rw = map.px_w() as usize;
    let rh = map.px_h() as usize;
    let mut fb = Framebuffer::new(rw, rh);
    let mut input = Input::new(kitty_kbd);

    let mut out: Vec<u8> = Vec::new();
    let mut b64: Vec<u8> = Vec::new();

    let sim_dt = 1.0 / 60.0;
    let sim_dt_ns = NS_PER_SEC / 60;
    let spin_margin = 1_000_000u64; // 1ms
    let mut acc: u64 = 0;
    let mut prev_t = now_ns();
    let mut next = now_ns();
    let mut prev_pos = player.pos;

    loop {
        if terminal::quit_requested() {
            break;
        }
        input.poll();
        if input.quit {
            break;
        }

        let now = now_ns();
        let mut elapsed = now - prev_t;
        prev_t = now;
        if elapsed > 8 * sim_dt_ns {
            elapsed = 8 * sim_dt_ns;
        }
        acc += elapsed;

        let mut jp = input.jump_pressed();
        while acc >= sim_dt_ns {
            prev_pos = player.pos;
            player.step(
                &map,
                sim_dt,
                input.axis_x(),
                jp,
                input.jump_held(),
                input.down_held(),
                &fp,
            );
            jp = false;
            acc -= sim_dt_ns;
            if player.pos.y > map.px_h() + 64.0 {
                player = Player::new(map.spawn.0, map.spawn.1);
                prev_pos = player.pos;
            }
        }

        if terminal::take_resize() {
            // We render at a fixed internal resolution; just repaint a clean bg.
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(b"\x1b[2J");
            let _ = o.flush();
        }

        let alpha = acc as f64 / sim_dt_ns as f64;
        let rpos = prev_pos.lerp(player.pos, alpha);
        render(&mut fb, &map, rpos, &player);
        kitty::present_rgba(&mut out, rw, rh, &fb.px, &mut b64);
        {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(&out);
            let _ = o.flush();
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

    eprintln!("== all scenarios passed ==");
}

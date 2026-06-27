//! The Zoomies run loop: auto-run right across the rooftop course at ever-rising
//! speed, with an autonomous (player-following, never-backtracking) camera over a
//! fixed horizon. Jump is the only input. Falling into a gap ends the run.
//!
//! The twist: graphics fidelity is the health bar. You start at tier 4 (Kitty
//! pixels); touching a rooftop hazard tears the renderer down one tier
//! (Kitty → half-blocks → ASCII → mono). A hit at mono (tier 1) ends the run, as
//! does falling between buildings.

use crate::gen::{self, Obstacle, Treat};
use crate::Difficulty;
use scamper::backend::{AsciiBackend, Backend, KittyBackend, MonoBackend, Overlay, TextBackend};
use scamper::framebuffer::{Framebuffer, Rgba};
use scamper::input::{Input, K_ESC, K_Q, K_SPACE, K_UP, K_W};
use scamper::level::art;
use scamper::level::View;
use scamper::mob::aabb_overlap;
use scamper::munchii;
use scamper::player::{FeelParams, Player};
use scamper::shake::Shake;
use scamper::sim::{Sim, SIM_DT_NS};
use scamper::time::{now_ns, sleep_until_ns, NS_PER_SEC};
use scamper::world::TILE;
use std::io::Write;

// Framebuffer px per terminal cell (keeps tile/cell dimensional parity, like game 1).
const CELL_PX: usize = 4;
const CELL_PH: usize = 8;
const MAX_INTERNAL_DIM: usize = 384;
/// Course length in tiles (finite-but-long; effectively endless at runner speeds).
const COURSE_TILES: i32 = 4000;
/// Invulnerability after a hit (~1.25s of i-frames at 60 Hz).
const HIT_GRACE: u32 = 75;

/// How a run ended.
pub enum Outcome {
    /// Fell into a gap after `distance` metres.
    Fell { distance: u32 },
    /// Took a hit at mono (tier 1) — out of fidelity.
    Downed { distance: u32 },
    /// Reached the end of the course.
    Maxed { distance: u32 },
    /// Player quit back to the menu mid-run.
    Quit { distance: u32 },
}

impl Outcome {
    pub fn distance(&self) -> u32 {
        match *self {
            Outcome::Fell { distance }
            | Outcome::Downed { distance }
            | Outcome::Maxed { distance }
            | Outcome::Quit { distance } => distance,
        }
    }
}

/// What one sim tick produced.
enum Tick {
    Continue,
    Hit,
    Heal,
    Ended(Outcome),
}

/// The backend for a fidelity tier: 4 = Kitty pixels, 3 = half-blocks, 2 = ASCII,
/// 1 = mono. (Tier 0 is death — never rendered.)
fn backend_for_tier(tier: u8) -> Box<dyn Backend> {
    match tier {
        4 => Box::new(KittyBackend::new()),
        3 => Box::new(TextBackend::new()),
        2 => Box::new(AsciiBackend::new()),
        _ => Box::new(MonoBackend::new()),
    }
}

/// Apply a hit to a fidelity tier: returns `(new_tier, dead)`. A hit at tier 1
/// (mono) drops to 0 = death.
fn hit_result(fidelity: u8) -> (u8, bool) {
    let new = fidelity.saturating_sub(1);
    (new, new == 0)
}

/// The HUD fidelity bar: filled pips for current tier, hollow for lost ones.
fn fidelity_pips(fidelity: u8) -> String {
    let f = fidelity.min(4) as usize;
    "●".repeat(f) + &"○".repeat(4 - f)
}

/// Viewport geometry: an internal framebuffer sized to a whole number of tiles, plus
/// the terminal cell area it scales across. The course is generated exactly this many
/// rows tall, so the horizon is fixed (cam_y = 0, no vertical scroll).
fn play_view(ws: scamper::terminal::WinSize) -> (usize, usize, u16, u16) {
    let tile = TILE as usize;
    let cpt_x = tile / CELL_PX;
    let cpt_y = tile / CELL_PH;
    let max_tiles = MAX_INTERNAL_DIM / tile;
    let view_tw = (ws.cols.max(20) as usize / cpt_x).clamp(6, max_tiles);
    let view_th = ((ws.rows.max(6) as usize - 1) / cpt_y).clamp(5, max_tiles);
    (view_tw * tile, view_th * tile, (view_tw * cpt_x) as u16, (view_th * cpt_y) as u16)
}

/// Play one run. `seed` makes the course reproducible. Returns how it ended.
pub fn run(input: &mut Input, difficulty: Difficulty, seed: u64) -> Outcome {
    let ws = scamper::terminal::query_winsize();
    let (fb_w, fb_h, cols, rows) = play_view(ws);
    let rows_tiles = (fb_h / TILE as usize) as i32;

    let course = gen::generate(seed, difficulty, COURSE_TILES, rows_tiles);
    let base_fp = gen::base_feel();
    let mut sim = Sim::new(Player::new(course.spawn.0, course.spawn.1), course.spawn);
    sim.player.vel.x = gen::run_speed(difficulty, course.spawn.0);
    sim.player.facing = 1;

    let mut fidelity: u8 = 4;
    let mut invuln: u32 = 0;
    let mut backend = backend_for_tier(fidelity);
    let mut fb = Framebuffer::new(fb_w, fb_h);
    let mut out: Vec<u8> = Vec::new();

    let left_margin = fb_w as f64 * 0.30;
    let mut cam_x = (course.spawn.0 - left_margin).max(0.0);
    let course_end_px = course.width_tiles as f64 * TILE;
    let death_y = rows_tiles as f64 * TILE;

    let mut treats = course.treats.clone(); // live list; collected ones are removed
    let mut pending_jump = false;
    let mut full_redraw = true;
    let mut acc: u64 = 0;
    let mut prev = now_ns();
    let mut shake = Shake::new();
    let mut frame: u64 = 0;

    let outcome = 'game: loop {
        input.poll();
        if scamper::terminal::quit_requested() || input.quit || input.pressed(K_Q) || input.pressed(K_ESC) {
            break 'game Outcome::Quit { distance: distance_m(&sim, &course) };
        }
        if input.pressed(K_SPACE) || input.pressed(K_UP) || input.pressed(K_W) {
            pending_jump = true;
        }
        let jump_held = input.held(K_SPACE) || input.held(K_UP) || input.held(K_W);
        let down_held = input.down_held();

        let now = now_ns();
        let mut elapsed = now - prev;
        prev = now;
        if elapsed > 8 * SIM_DT_NS {
            elapsed = 8 * SIM_DT_NS;
        }
        acc += elapsed;
        while acc >= SIM_DT_NS {
            let tick = advance(&mut sim, &course, &mut treats, difficulty, base_fp, pending_jump, jump_held, down_held, death_y, course_end_px, &mut invuln);
            pending_jump = false;
            acc -= SIM_DT_NS;
            match tick {
                Tick::Ended(o) => {
                    if matches!(o, Outcome::Fell { .. }) {
                        draw_frame(&mut *backend, &mut out, &mut fb, &course, &treats, &sim, cam_x, (0.0, 0.0), cols, rows, fb_w, fb_h, true, fidelity, invuln, difficulty);
                    }
                    break 'game o;
                }
                Tick::Hit => {
                    let (nf, dead) = hit_result(fidelity);
                    fidelity = nf;
                    if dead {
                        break 'game Outcome::Downed { distance: distance_m(&sim, &course) };
                    }
                    // Tear the old renderer down and rebuild one tier lower, with a jolt.
                    swap_backend(&mut backend, &mut out, fidelity);
                    shake.bump(0.6);
                    full_redraw = true;
                }
                Tick::Heal => {
                    // A steak restores a fidelity tier — rebuild the renderer one step
                    // sharper (no-op flash if already at full Kitty).
                    if fidelity < 4 {
                        fidelity += 1;
                        swap_backend(&mut backend, &mut out, fidelity);
                        full_redraw = true;
                    }
                    shake.bump(0.3);
                }
                Tick::Continue => {}
            }
        }

        // Camera: follow the player, never backtrack (forced rightward).
        cam_x = cam_x.max(sim.player.pos.x - left_margin).max(0.0);

        let sh = shake.offset(frame, 5.0);
        frame += 1;
        let needs_redraw = full_redraw || shake.active();
        draw_frame(&mut *backend, &mut out, &mut fb, &course, &treats, &sim, cam_x, sh, cols, rows, fb_w, fb_h, needs_redraw, fidelity, invuln, difficulty);
        full_redraw = false;
        sleep_until_ns(now_ns() + 16_000_000, 1_000_000);
    };
    // Tear the run's backend down (delete Kitty images, clear) so the caller's score
    // card / menu paints over a clean screen.
    let mut o2: Vec<u8> = Vec::new();
    backend.teardown(&mut o2);
    {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(&o2);
        let _ = o.write_all(b"\x1b[2J");
        let _ = o.flush();
    }
    outcome
}

/// Swap to `tier`'s backend: tear the current one down, clear, and replace.
fn swap_backend(backend: &mut Box<dyn Backend>, out: &mut Vec<u8>, tier: u8) {
    let mut o2: Vec<u8> = Vec::new();
    backend.teardown(&mut o2);
    {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(&o2);
        let _ = o.write_all(b"\x1b[2J");
        let _ = o.flush();
    }
    *backend = backend_for_tier(tier);
    out.clear();
}

fn distance_m(sim: &Sim, course: &gen::Course) -> u32 {
    ((sim.player.pos.x - course.spawn.0).max(0.0) / TILE) as u32
}

/// Advance one sim tick: ramp speed, drive auto-run + jump, count down i-frames, then
/// check hazards and the end conditions. Pure of rendering, so it's headless-testable.
#[allow(clippy::too_many_arguments)]
fn advance(
    sim: &mut Sim,
    course: &gen::Course,
    treats: &mut Vec<Treat>,
    difficulty: Difficulty,
    base_fp: FeelParams,
    jump_pressed: bool,
    jump_held: bool,
    down_held: bool,
    death_y: f64,
    course_end_px: f64,
    invuln: &mut u32,
) -> Tick {
    *invuln = invuln.saturating_sub(1);

    let v = gen::run_speed(difficulty, sim.player.pos.x);
    let mut fp = base_fp;
    fp.max_run = v;
    fp.run_accel = base_fp.run_accel.max(v * 2.0);
    sim.fp = fp;
    let inp = scamper::capture::InputFrame { axis_x: 1, jump_pressed, jump_held, down_held };
    sim.step(&course.map, inp);

    let p = &sim.player;
    if p.pos.y > death_y {
        return Tick::Ended(Outcome::Fell { distance: distance_m(sim, course) });
    }
    if p.pos.x + p.w >= course_end_px {
        return Tick::Ended(Outcome::Maxed { distance: distance_m(sim, course) });
    }
    // Grab a steak (any tick, even mid-i-frames) → heal a tier.
    if let Some(i) = treats.iter().position(|t| {
        (t.x - p.pos.x).abs() < 64.0 && aabb_overlap(p.pos.x, p.pos.y, p.w, p.h, t.x, t.y, t.w, t.h)
    }) {
        treats.swap_remove(i);
        return Tick::Heal;
    }
    if *invuln == 0 && hits_obstacle(p, &course.obstacles) {
        *invuln = HIT_GRACE;
        return Tick::Hit;
    }
    Tick::Continue
}

/// Does the player's box overlap any hazard? (Only checks ones near its x.)
fn hits_obstacle(p: &Player, obstacles: &[Obstacle]) -> bool {
    obstacles.iter().any(|o| {
        (o.x - p.pos.x).abs() < 64.0 && aabb_overlap(p.pos.x, p.pos.y, p.w, p.h, o.x, o.y, o.w, o.h)
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_frame(
    backend: &mut dyn Backend,
    out: &mut Vec<u8>,
    fb: &mut Framebuffer,
    course: &gen::Course,
    treats: &[Treat],
    sim: &Sim,
    cam_x_in: f64,
    shake: (f64, f64),
    cols: u16,
    rows: u16,
    fb_w: usize,
    fb_h: usize,
    full_redraw: bool,
    fidelity: u8,
    invuln: u32,
    difficulty: Difficulty,
) {
    out.clear(); // the backends append this frame's bytes; we flush them below
    let pal = art::palette(art::Theme::Rooftop);
    let cpw = fb_w as f64 / cols.max(1) as f64;
    let cph = fb_h as f64 / rows.max(1) as f64;

    // Shake nudges the camera before snapping, so it trembles by whole pixels/cells.
    let mut view = View { cam_x: cam_x_in + shake.0, cam_y: shake.1, zoom: 1, view_w: fb_w, view_h: fb_h };
    if backend.pixel_exact() {
        view.snap_pixels();
    } else {
        view.snap_cells(cpw, cph);
    }
    let cam_x = view.cam_x;
    let sx = |wx: f64| view.sx(wx);
    let sy = |wy: f64| view.sy(wy);

    // Backdrop (screen-constant skyline), then the solid rooftops over it.
    fb.clear(pal.sky);
    art::draw_backdrop(fb, art::Theme::Rooftop, &pal, cam_x, fb_w, fb_h);
    let t = TILE as i32;
    let tx0 = (cam_x / TILE).floor() as i32;
    let tx1 = ((cam_x + fb_w as f64) / TILE).ceil() as i32;
    for ty in 0..course.rows {
        for tx in tx0..tx1 {
            if course.map.is_solid(tx, ty) {
                art::draw_tile(fb, tx * t - cam_x as i32, ty * t, scamper::level::ir::TileKind::Ground, &pal);
            }
        }
    }

    // Sprite list: hazards first, then the runner on top. (lines, top-left px, palette)
    type Drawable = (Vec<String>, f64, f64, fn(char) -> (u8, u8, u8));
    let mut sprites: Vec<Drawable> = Vec::new();

    if let Some(sp) = scamper::sprite::get("prickle") {
        let an = sp.anim("walk");
        let frame = &an.frames[0];
        for o in &course.obstacles {
            if o.x < cam_x - TILE || o.x > cam_x + fb_w as f64 + TILE {
                continue;
            }
            let lines: Vec<String> = frame.iter().map(|s| s.to_string()).collect();
            let fw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
            let (mw, mh) = (fw * cpw, lines.len() as f64 * cph);
            let lx = sx(o.x + o.w / 2.0) - mw / 2.0;
            let ly = sy(o.y + o.h) - mh;
            sprites.push((lines, lx, ly, sp.palette));
        }
    }

    // Steak treats (restore a fidelity tier on touch).
    if let Some(sp) = scamper::sprite::get("steak") {
        let an = sp.anim("idle");
        let n = an.frames.len().max(1);
        let fi = (sim.clock() / (NS_PER_SEC / an.fps.max(1) as u64)) as usize % n;
        let frame = &an.frames[fi];
        for tr in treats {
            if tr.x < cam_x - TILE || tr.x > cam_x + fb_w as f64 + TILE {
                continue;
            }
            let lines: Vec<String> = frame.iter().map(|s| s.to_string()).collect();
            let fw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
            let (mw, mh) = (fw * cpw, lines.len() as f64 * cph);
            let lx = sx(tr.x + tr.w / 2.0) - mw / 2.0;
            let ly = sy(tr.y + tr.h) - mh;
            sprites.push((lines, lx, ly, sp.palette));
        }
    }

    // The runner: feet on the hitbox bottom, centered, always facing right. Blinks
    // during post-hit i-frames.
    let blink = invuln > 0 && (invuln / 4) % 2 == 0;
    if !blink {
        let anim = munchii::anim(if sim.player.grounded { "walk" } else { "jump" });
        let n = anim.frames.len().max(1);
        let fi = (sim.clock() / (NS_PER_SEC / anim.fps.max(1) as u64)) as usize % n;
        let lines: Vec<String> = anim.frames[fi].iter().map(|s| s.to_string()).collect();
        let fw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
        let (mw, mh) = (fw * cpw, lines.len() as f64 * cph);
        let lx = sx(sim.player.pos.x + sim.player.w / 2.0) - mw / 2.0;
        let ly = sy(sim.player.pos.y + sim.player.h) - mh;
        sprites.push((lines, lx, ly, munchii::beagle_rgb as fn(char) -> (u8, u8, u8)));
    }

    if backend.draws_overlay() {
        let overlays: Vec<Overlay> = sprites
            .iter()
            .enumerate()
            .map(|(i, (lns, lx, ly, p))| Overlay {
                lines: lns,
                col: (lx / cpw).round() as i32,
                row: (ly / cph).round() as i32,
                tint: None,
                palette: Some(*p),
                z: i as i32,
            })
            .collect();
        backend.present(out, fb, cols, rows, full_redraw, &overlays);
    } else {
        for (lns, lx, ly, p) in &sprites {
            draw_sprite_pixels(fb, lns, *lx, *ly, cpw, cph, *p);
        }
        backend.present(out, fb, cols, rows, full_redraw, &[]);
    }

    // Flush the encoded frame to the terminal (present only appended it to `out`).
    {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(out);
        let _ = o.flush();
    }

    // HUD on the row just below the play image (not over it — avoids fighting the
    // Kitty image at the top, which flickers on recomposite).
    draw_hud(distance_m(sim, course), gen::run_speed(difficulty, sim.player.pos.x), fidelity, cols, rows + 1);
}

/// A single status row (distance, speed, fidelity bar) on `status_row`, full width.
fn draw_hud(dist: u32, speed: f64, fidelity: u8, cols: u16, status_row: u16) {
    let text = format!("  ⚡ {dist} m    {:.0} px/s    fidelity {}  ", speed, fidelity_pips(fidelity));
    let text: String = text.chars().take(cols as usize).collect();
    let mut o = std::io::stdout().lock();
    let _ = write!(o, "\x1b[{};1H\x1b[2K\x1b[7m{text}\x1b[0m", status_row);
    let _ = o.flush();
}

/// Rasterize a glyph sprite into the framebuffer (pixel tiers). Spaces transparent.
fn draw_sprite_pixels(fb: &mut Framebuffer, lines: &[String], lx: f64, ly: f64, cpw: f64, cph: f64, palette: fn(char) -> (u8, u8, u8)) {
    for (gr, line) in lines.iter().enumerate() {
        for (gc, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let (r, g, b) = palette(ch);
            let x0 = (lx + gc as f64 * cpw).floor() as i32;
            let x1 = (lx + (gc as f64 + 1.0) * cpw).floor() as i32;
            let y0 = (ly + gr as f64 * cph).floor() as i32;
            let y1 = (ly + (gr as f64 + 1.0) * cph).floor() as i32;
            fb.fill_rect(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1), Rgba::rgb(r, g, b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fidelity_ladder_maps_tiers_to_backends() {
        assert_eq!(backend_for_tier(4).name(), "kitty");
        assert_eq!(backend_for_tier(3).name(), "text");
        assert_eq!(backend_for_tier(2).name(), "ascii");
        assert_eq!(backend_for_tier(1).name(), "mono");
    }

    #[test]
    fn hits_walk_down_the_tiers_then_kill() {
        let mut f = 4u8;
        let mut deaths = 0;
        for _ in 0..4 {
            let (nf, dead) = hit_result(f);
            f = nf;
            if dead {
                deaths += 1;
            }
        }
        assert_eq!(f, 0, "four hits exhaust fidelity");
        assert_eq!(deaths, 1, "only the hit at tier 1 is fatal");
        assert_eq!(fidelity_pips(4), "●●●●");
        assert_eq!(fidelity_pips(1), "●○○○");
    }

    /// Headless harness mirroring `run`'s setup, looping `advance` with a jump policy.
    fn simulate(seed: u64, difficulty: Difficulty, rows_tiles: i32, jump: impl Fn(&Sim, &gen::Course) -> bool) -> (Outcome, u32) {
        let course = gen::generate(seed, difficulty, 4000, rows_tiles);
        let base_fp = gen::base_feel();
        let mut sim = Sim::new(Player::new(course.spawn.0, course.spawn.1), course.spawn);
        sim.player.vel.x = gen::run_speed(difficulty, course.spawn.0);
        sim.player.facing = 1;
        let death_y = rows_tiles as f64 * TILE;
        let course_end_px = course.width_tiles as f64 * TILE;
        let mut fidelity = 4u8;
        let mut invuln = 0u32;
        let mut treats = course.treats.clone();
        for tick in 0..6000u32 {
            let jp = jump(&sim, &course);
            match advance(&mut sim, &course, &mut treats, difficulty, base_fp, jp, jp, false, death_y, course_end_px, &mut invuln) {
                Tick::Ended(o) => return (o, tick),
                Tick::Hit => {
                    let (nf, dead) = hit_result(fidelity);
                    fidelity = nf;
                    if dead {
                        return (Outcome::Downed { distance: distance_m(&sim, &course) }, tick);
                    }
                }
                Tick::Heal => fidelity = (fidelity + 1).min(4),
                Tick::Continue => {}
            }
        }
        (Outcome::Quit { distance: distance_m(&sim, &course) }, 6000)
    }

    #[test]
    fn spawns_on_a_solid_roof_and_auto_runs() {
        // With NO jumping, the runner clears some flat opening roof, then falls at the
        // first gap (the opening building has no hazard) — proving spawn-on-solid,
        // auto-run, and gap-death detection.
        let (outcome, _) = simulate(7, Difficulty::Standard, 14, |_, _| false);
        match outcome {
            Outcome::Fell { distance } => {
                assert!(distance >= 3, "should clear some opening roof first, got {distance} m");
                assert!(distance < 60, "without jumping it can't get far, got {distance} m");
            }
            other => panic!("expected a fall without jumping, got {} m", other.distance()),
        }
    }

    #[test]
    fn grabbing_a_steak_signals_a_heal_and_consumes_it() {
        let course = gen::generate(11, Difficulty::Standard, 2000, 24);
        let mut treats = course.treats.clone();
        assert!(!treats.is_empty(), "need a steak to test");
        let t0 = treats[0];
        // Drop the runner right onto the steak.
        let mut sim = Sim::new(Player::new(t0.x, t0.y), (t0.x, t0.y));
        let before = treats.len();
        let mut invuln = 0u32;
        let r = advance(&mut sim, &course, &mut treats, Difficulty::Standard, gen::base_feel(), false, false, false, 24.0 * TILE, 1.0e9, &mut invuln);
        assert!(matches!(r, Tick::Heal), "touching a steak heals");
        assert_eq!(treats.len(), before - 1, "the steak is consumed");
    }

    #[test]
    fn distance_is_monotonic_while_running() {
        let course = gen::generate(1, Difficulty::Cruise, 4000, 14);
        let base_fp = gen::base_feel();
        let mut sim = Sim::new(Player::new(course.spawn.0, course.spawn.1), course.spawn);
        sim.player.vel.x = gen::run_speed(Difficulty::Cruise, course.spawn.0);
        let mut invuln = 0u32;
        let mut treats = course.treats.clone();
        let mut last = sim.player.pos.x;
        for _ in 0..30 {
            advance(&mut sim, &course, &mut treats, Difficulty::Cruise, base_fp, false, false, false, 14.0 * TILE, 4000.0 * TILE, &mut invuln);
            assert!(sim.player.pos.x >= last - 0.001, "x went backwards: {} < {}", sim.player.pos.x, last);
            last = sim.player.pos.x;
        }
    }
}

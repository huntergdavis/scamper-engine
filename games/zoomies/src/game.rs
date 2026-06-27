//! The Zoomies run loop: auto-run right across the rooftop course at ever-rising
//! speed, with an autonomous (player-following, never-backtracking) camera over a
//! fixed horizon. Jump is the only input. Falling into a gap ends the run.
//!
//! Hits + the fidelity-as-health backend swap come in the next step; here the run is
//! Kitty-only and ends on a fall or on reaching the end of the course.

use crate::gen;
use crate::Difficulty;
use scamper::backend::{Backend, KittyBackend, Overlay};
use scamper::framebuffer::{Framebuffer, Rgba};
use scamper::input::{Input, K_ESC, K_Q, K_SPACE, K_UP, K_W};
use scamper::level::art;
use scamper::level::View;
use scamper::munchii;
use scamper::player::Player;
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

/// How a run ended.
pub enum Outcome {
    /// Fell into a gap (or was crushed) after `distance` metres.
    Fell { distance: u32 },
    /// Reached the end of the course.
    Maxed { distance: u32 },
    /// Player quit back to the menu mid-run.
    Quit { distance: u32 },
}

impl Outcome {
    pub fn distance(&self) -> u32 {
        match *self {
            Outcome::Fell { distance } | Outcome::Maxed { distance } | Outcome::Quit { distance } => distance,
        }
    }
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

/// Play one run. `seed` makes the course reproducible (the caller passes a fresh
/// wall-clock seed for variety; tests can pin it). Returns how it ended + distance.
pub fn run(input: &mut Input, difficulty: Difficulty, seed: u64) -> Outcome {
    let ws = scamper::terminal::query_winsize();
    let (fb_w, fb_h, cols, rows) = play_view(ws);
    let rows_tiles = (fb_h / TILE as usize) as i32;

    let course = gen::generate(seed, difficulty, COURSE_TILES, rows_tiles);
    let base_fp = gen::base_feel();
    let mut sim = Sim::new(Player::new(course.spawn.0, course.spawn.1), course.spawn);
    sim.player.vel.x = gen::run_speed(difficulty, course.spawn.0); // start at run speed
    sim.player.facing = 1;

    let mut backend: Box<dyn Backend> = Box::new(KittyBackend::new());
    let mut fb = Framebuffer::new(fb_w, fb_h);
    let mut out: Vec<u8> = Vec::new();

    let left_margin = fb_w as f64 * 0.30;
    let mut cam_x = (course.spawn.0 - left_margin).max(0.0);
    let course_end_px = course.width_tiles as f64 * TILE;
    let death_y = rows_tiles as f64 * TILE; // fell below the rooftops

    let mut pending_jump = false;
    let mut full_redraw = true;
    let mut acc: u64 = 0;
    let mut prev = now_ns();

    loop {
        input.poll();
        if scamper::terminal::quit_requested() || input.quit || input.pressed(K_Q) || input.pressed(K_ESC) {
            return Outcome::Quit { distance: distance_m(&sim, &course) };
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
            elapsed = 8 * SIM_DT_NS; // clamp a long stall so we don't spiral
        }
        acc += elapsed;
        while acc >= SIM_DT_NS {
            let outcome = advance(&mut sim, &course, difficulty, base_fp, pending_jump, jump_held, down_held, death_y, course_end_px);
            pending_jump = false;
            acc -= SIM_DT_NS;
            if let Some(o) = outcome {
                if matches!(o, Outcome::Fell { .. }) {
                    // Hold on the fallen frame for a beat so the fall reads.
                    draw_frame(&mut *backend, &mut out, &mut fb, &course, &sim, cam_x, cols, rows, fb_w, fb_h, true, down_held, difficulty);
                }
                return o;
            }
        }

        // Camera: follow the player, never backtrack (forced rightward).
        cam_x = cam_x.max(sim.player.pos.x - left_margin).max(0.0);

        draw_frame(&mut *backend, &mut out, &mut fb, &course, &sim, cam_x, cols, rows, fb_w, fb_h, full_redraw, down_held, difficulty);
        full_redraw = false;
        sleep_until_ns(now_ns() + 16_000_000, 1_000_000);
    }
}

fn distance_m(sim: &Sim, course: &gen::Course) -> u32 {
    ((sim.player.pos.x - course.spawn.0).max(0.0) / TILE) as u32
}

/// Advance one sim tick of the run: ramp speed, drive auto-run + the jump input, then
/// check the end conditions. Returns `Some(outcome)` when the run is over. Pure of
/// any rendering, so it's headless-testable.
#[allow(clippy::too_many_arguments)]
fn advance(
    sim: &mut Sim,
    course: &gen::Course,
    difficulty: Difficulty,
    base_fp: scamper::player::FeelParams,
    jump_pressed: bool,
    jump_held: bool,
    down_held: bool,
    death_y: f64,
    course_end_px: f64,
) -> Option<Outcome> {
    // Auto-run: drive +x and ramp max_run to the speed for this position.
    let v = gen::run_speed(difficulty, sim.player.pos.x);
    let mut fp = base_fp;
    fp.max_run = v;
    fp.run_accel = base_fp.run_accel.max(v * 2.0); // reach speed promptly after a landing
    sim.fp = fp;
    let inp = scamper::capture::InputFrame { axis_x: 1, jump_pressed, jump_held, down_held };
    sim.step(&course.map, inp);

    if sim.player.pos.y > death_y {
        return Some(Outcome::Fell { distance: distance_m(sim, course) });
    }
    if sim.player.pos.x + sim.player.w >= course_end_px {
        return Some(Outcome::Maxed { distance: distance_m(sim, course) });
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn draw_frame(
    backend: &mut dyn Backend,
    out: &mut Vec<u8>,
    fb: &mut Framebuffer,
    course: &gen::Course,
    sim: &Sim,
    cam_x_in: f64,
    cols: u16,
    rows: u16,
    fb_w: usize,
    fb_h: usize,
    full_redraw: bool,
    down_held: bool,
    difficulty: Difficulty,
) {
    let pal = art::palette(art::Theme::Rooftop);
    let cpw = fb_w as f64 / cols.max(1) as f64;
    let cph = fb_h as f64 / rows.max(1) as f64;

    let mut view = View { cam_x: cam_x_in, cam_y: 0.0, zoom: 1, view_w: fb_w, view_h: fb_h };
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

    // The runner sprite: feet on the hitbox bottom, centered, always facing right.
    let anim = munchii::anim(if sim.player.grounded { "walk" } else { "jump" });
    let n = anim.frames.len().max(1);
    let fi = (sim.clock() / (NS_PER_SEC / anim.fps.max(1) as u64)) as usize % n;
    let lines: Vec<String> = anim.frames[fi].iter().map(|s| s.to_string()).collect();
    let _ = down_held;
    let fw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
    let (mw, mh) = (fw * cpw, lines.len() as f64 * cph);
    let lx = sx(sim.player.pos.x + sim.player.w / 2.0) - mw / 2.0;
    let ly = sy(sim.player.pos.y + sim.player.h) - mh;

    if backend.draws_overlay() {
        let overlays = [Overlay {
            lines: &lines,
            col: (lx / cpw).round() as i32,
            row: (ly / cph).round() as i32,
            tint: None,
            palette: Some(munchii::beagle_rgb as fn(char) -> (u8, u8, u8)),
            z: 0,
        }];
        backend.present(out, fb, cols, rows, full_redraw, &overlays);
    } else {
        draw_sprite_pixels(fb, &lines, lx, ly, cpw, cph, munchii::beagle_rgb);
        backend.present(out, fb, cols, rows, full_redraw, &[]);
    }

    draw_hud(distance_m(sim, course), gen::run_speed(difficulty, sim.player.pos.x), cols);
}

/// A single status row painted on top (distance, speed, fidelity pips placeholder).
fn draw_hud(dist: u32, speed: f64, cols: u16) {
    let pips = "●●●●"; // fidelity bar lands with hits in the next step
    let text = format!("  ⚡ {dist} m    {:.0} px/s    fidelity {pips}  ", speed);
    let text: String = text.chars().take(cols as usize).collect();
    let mut o = std::io::stdout().lock();
    let _ = write!(o, "\x1b[1;1H\x1b[7m{text}\x1b[0m");
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

    /// Headless harness mirroring `run`'s setup, looping `advance` with a fixed jump
    /// policy — no terminal/render. Returns (outcome, ticks).
    fn simulate(seed: u64, difficulty: Difficulty, rows_tiles: i32, jump: impl Fn(&Sim, &gen::Course) -> bool) -> (Outcome, u32) {
        let course = gen::generate(seed, difficulty, 4000, rows_tiles);
        let base_fp = gen::base_feel();
        let mut sim = Sim::new(Player::new(course.spawn.0, course.spawn.1), course.spawn);
        sim.player.vel.x = gen::run_speed(difficulty, course.spawn.0);
        sim.player.facing = 1;
        let death_y = rows_tiles as f64 * TILE;
        let course_end_px = course.width_tiles as f64 * TILE;
        for tick in 0..6000u32 {
            let jp = jump(&sim, &course);
            if let Some(o) = advance(&mut sim, &course, difficulty, base_fp, jp, jp, false, death_y, course_end_px) {
                return (o, tick);
            }
        }
        (Outcome::Quit { distance: distance_m(&sim, &course) }, 6000)
    }

    #[test]
    fn spawns_on_a_solid_roof_and_auto_runs() {
        // With NO jumping, the runner should cross several tiles of the flat opening
        // roof before reaching the first gap and falling — proving it spawns on solid
        // ground, auto-runs right, and gap-death is detected (not an instant fall).
        let (outcome, _) = simulate(7, Difficulty::Standard, 14, |_, _| false);
        match outcome {
            Outcome::Fell { distance } => {
                assert!(distance >= 3, "should clear some opening roof first, got {distance} m");
                assert!(distance < 60, "without jumping it can't get far, got {distance} m");
            }
            other => panic!("expected a fall without jumping, got {} m via other outcome", other.distance()),
        }
    }

    #[test]
    fn distance_is_monotonic_while_running() {
        // Sanity: auto-run never moves the player left over the opening stretch.
        let course = gen::generate(1, Difficulty::Cruise, 4000, 14);
        let base_fp = gen::base_feel();
        let mut sim = Sim::new(Player::new(course.spawn.0, course.spawn.1), course.spawn);
        sim.player.vel.x = gen::run_speed(Difficulty::Cruise, course.spawn.0);
        let mut last = sim.player.pos.x;
        for _ in 0..30 {
            advance(&mut sim, &course, Difficulty::Cruise, base_fp, false, false, false, 14.0 * TILE, 4000.0 * TILE);
            assert!(sim.player.pos.x >= last - 0.001, "x went backwards: {} < {}", sim.player.pos.x, last);
            last = sim.player.pos.x;
        }
    }
}

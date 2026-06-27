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
use scamper::mob::{aabb_overlap, Gait, Mob};
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

/// How a run ended, carrying the final `score` (distance boosted by the pristine
/// multiplier — see the run loop).
pub enum Outcome {
    /// Fell into a gap.
    Fell { score: u32 },
    /// Took a hit at mono (tier 1) — out of fidelity.
    Downed { score: u32 },
    /// Reached the end of the course.
    Maxed { score: u32 },
    /// Player quit back to the menu mid-run.
    Quit { score: u32 },
}

impl Outcome {
    pub fn score(&self) -> u32 {
        match *self {
            Outcome::Fell { score } | Outcome::Downed { score } | Outcome::Maxed { score } | Outcome::Quit { score } => score,
        }
    }
}

/// Why a tick ended the run (the score is added by the run loop, which owns it).
enum EndReason {
    Fell,
    Maxed,
}

/// What one sim tick produced.
enum Tick {
    Continue,
    Hit,
    Heal,
    Ended(EndReason),
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

/// Tag colors: green for a heal "+", red for a hazard "-". (Shape carries it in mono.)
fn plus_rgb(_: char) -> (u8, u8, u8) {
    (120, 240, 140)
}
fn minus_rgb(_: char) -> (u8, u8, u8) {
    (240, 110, 100)
}

/// The pristine multiplier for a full-fidelity streak length (ticks): 1× ramping to
/// a 3× cap over ~10s of clean (tier-4) play. A hit resets the streak.
fn pristine_mult(pristine_ticks: u32) -> f64 {
    1.0 + (pristine_ticks as f64 / 600.0).min(2.0)
}

/// World px per time-of-day phase (~250 tiles), cycling night→dawn→day→dusk.
const PHASE_LEN_PX: f64 = 4000.0;
/// Sky keyframes for the cycle, in phase order.
const SKIES: [(u8, u8, u8); 4] = [(18, 20, 40), (64, 42, 74), (74, 110, 150), (96, 52, 58)];

fn phase_name(phase: u32) -> &'static str {
    match phase % 4 {
        0 => "NIGHT",
        1 => "DAWN",
        2 => "DAY",
        _ => "DUSK",
    }
}

/// Re-tint a palette's sky for the time of day at world x — the backdrop (which is
/// sky + a faint bump) rides along, so the skyline glows dawn/day/dusk as you run.
fn time_palette(mut p: art::Palette, x_px: f64) -> art::Palette {
    let t = (x_px / PHASE_LEN_PX).max(0.0);
    let (i, f) = ((t as usize) % 4, t.fract());
    let (a, b) = (SKIES[i], SKIES[(i + 1) % 4]);
    let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f) as u8;
    p.sky = Rgba::rgb(lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2));
    p
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

/// Build the live bird mobs from a course: strafers slide at roof level (Track),
/// divers bob vertically at a fixed x, dipping to the roof (Swoop). Speed 0 — Track
/// oscillates on its own and a still Swoop is a pure vertical dive.
fn build_birds(course: &gen::Course) -> Vec<Mob> {
    course
        .birds
        .iter()
        .map(|b| {
            let (y, gait) = if b.dive {
                (b.roof as f64 * TILE - 34.0, Gait::Swoop)
            } else {
                (b.roof as f64 * TILE - 12.0, Gait::Track)
            };
            Mob::new(b.home_x, y, 14.0, 12.0, -1, 0.0, gait)
        })
        .collect()
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
    // Patrol birds: Track gait oscillates them ±~3 tiles around their home x.
    let mut birds = build_birds(&course);
    let mut score = 0.0f64; // distance, boosted by the pristine multiplier
    let mut pristine = 0u32; // ticks held at full fidelity (drives the multiplier)
    let mut last_phase = (sim.player.pos.x / PHASE_LEN_PX) as u32; // time-of-day phase
    let mut pending_jump = false;
    let mut full_redraw = true;
    let mut acc: u64 = 0;
    let mut prev = now_ns();
    let mut shake = Shake::new();
    let mut frame: u64 = 0;

    let outcome = 'game: loop {
        input.poll();
        if scamper::terminal::quit_requested() || input.quit || input.pressed(K_Q) || input.pressed(K_ESC) {
            break 'game Outcome::Quit { score: score as u32 };
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
            let px0 = sim.player.pos.x;
            let tick = advance(&mut sim, &course, &mut treats, &mut birds, difficulty, base_fp, pending_jump, jump_held, down_held, death_y, course_end_px, &mut invuln);
            pending_jump = false;
            acc -= SIM_DT_NS;
            match tick {
                Tick::Ended(reason) => {
                    let s = score as u32;
                    if matches!(reason, EndReason::Fell) {
                        let mult = pristine_mult(pristine);
                        draw_frame(&mut *backend, &mut out, &mut fb, &course, &treats, &birds, &sim, cam_x, (0.0, 0.0), cols, rows, fb_w, fb_h, true, fidelity, invuln, difficulty, s, mult);
                        break 'game Outcome::Fell { score: s };
                    }
                    break 'game Outcome::Maxed { score: s };
                }
                Tick::Hit => {
                    let (nf, dead) = hit_result(fidelity);
                    fidelity = nf;
                    pristine = 0; // a hit breaks the streak → multiplier resets to 1×
                    if dead {
                        break 'game Outcome::Downed { score: score as u32 };
                    }
                    // Tear the old renderer down and rebuild one tier lower, with a jolt.
                    swap_backend(&mut backend, &mut out, fidelity);
                    shake.bump(0.6);
                    full_redraw = true;
                    let px = sim.player.pos.x + sim.player.w / 2.0;
                    let clk = sim.clock();
                    sim.fx.spawn(&scamper::effects::BANG, px, sim.player.pos.y - 4.0, clk);
                    sim.fx.spawn_word("OUCH!", (255, 120, 110), px, sim.player.pos.y - 12.0, clk);
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
                    let px = sim.player.pos.x + sim.player.w / 2.0;
                    let clk = sim.clock();
                    sim.fx.spawn(&scamper::effects::SPARKLE, px, sim.player.pos.y, clk);
                    sim.fx.spawn_word("✚ YUM", (120, 240, 140), px, sim.player.pos.y - 12.0, clk);
                }
                Tick::Continue => {}
            }
            // Pristine bonus: the multiplier grows only while at full fidelity, and
            // scores the distance covered this tick.
            if fidelity >= 4 {
                pristine += 1;
            } else {
                pristine = 0;
            }
            score += ((sim.player.pos.x - px0).max(0.0) / TILE) * pristine_mult(pristine);
        }

        // Time-of-day milestone: announce each new phase as you cross into it.
        let phase = (sim.player.pos.x / PHASE_LEN_PX) as u32;
        if phase != last_phase {
            last_phase = phase;
            let px = sim.player.pos.x + sim.player.w / 2.0;
            sim.fx.spawn_word(phase_name(phase), (210, 215, 255), px, sim.player.pos.y - 14.0, sim.clock());
        }

        // Camera: follow the player, never backtrack (forced rightward).
        cam_x = cam_x.max(sim.player.pos.x - left_margin).max(0.0);

        let sh = shake.offset(frame, 5.0);
        frame += 1;
        let needs_redraw = full_redraw || shake.active();
        draw_frame(&mut *backend, &mut out, &mut fb, &course, &treats, &birds, &sim, cam_x, sh, cols, rows, fb_w, fb_h, needs_redraw, fidelity, invuln, difficulty, score as u32, pristine_mult(pristine));
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


/// Advance one sim tick: ramp speed, drive auto-run + jump, count down i-frames, then
/// check hazards and the end conditions. Pure of rendering, so it's headless-testable.
#[allow(clippy::too_many_arguments)]
fn advance(
    sim: &mut Sim,
    course: &gen::Course,
    treats: &mut Vec<Treat>,
    birds: &mut [Mob],
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
    for b in birds.iter_mut() {
        b.step(&course.map); // Track gait: oscillate horizontally over the roof
    }

    let p = &sim.player;
    if p.pos.y > death_y {
        return Tick::Ended(EndReason::Fell);
    }
    if p.pos.x + p.w >= course_end_px {
        return Tick::Ended(EndReason::Maxed);
    }
    // Grab a steak (any tick, even mid-i-frames) → heal a tier.
    if let Some(i) = treats.iter().position(|t| {
        (t.x - p.pos.x).abs() < 64.0 && aabb_overlap(p.pos.x, p.pos.y, p.w, p.h, t.x, t.y, t.w, t.h)
    }) {
        treats.swap_remove(i);
        return Tick::Heal;
    }
    // A static hazard or a patrolling bird hurts (gated by i-frames).
    if *invuln == 0 && (hits_obstacle(p, &course.obstacles) || hits_bird(p, birds)) {
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

/// Does the player's box overlap a patrolling bird?
fn hits_bird(p: &Player, birds: &[Mob]) -> bool {
    birds.iter().any(|b| {
        (b.pos.x - p.pos.x).abs() < 64.0 && aabb_overlap(p.pos.x, p.pos.y, p.w, p.h, b.pos.x, b.pos.y, b.w, b.h)
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_frame(
    backend: &mut dyn Backend,
    out: &mut Vec<u8>,
    fb: &mut Framebuffer,
    course: &gen::Course,
    treats: &[Treat],
    birds: &[Mob],
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
    score: u32,
    mult: f64,
) {
    out.clear(); // the backends append this frame's bytes; we flush them below
    let pal = time_palette(art::palette(art::Theme::Rooftop), sim.player.pos.x);
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
            let cx = lx + mw / 2.0;
            sprites.push((lines, lx, ly, sp.palette));
            // A red "-" tag above it: this one HURTS (reads in B&W by shape alone).
            let bar = bigtext("-");
            let bw = bar[0].chars().count() as f64 * cpw;
            sprites.push((bar, cx - bw / 2.0, ly - 6.0 * cph, minus_rgb as fn(char) -> (u8, u8, u8)));
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
            let cx = lx + mw / 2.0;
            sprites.push((lines, lx, ly, sp.palette));
            // A green medical-cross marker, spelled in the pixel font so "good" reads
            // in black & white by shape (not just color). Gently bobs to catch the eye.
            let cross = bigtext("✚");
            let cw = cross[0].chars().count() as f64 * cpw;
            let bob = (sim.clock() as f64 / 1.0e9 * 5.0).sin() * cph * 0.6;
            sprites.push((cross, cx - cw / 2.0, ly - 6.0 * cph + bob, plus_rgb as fn(char) -> (u8, u8, u8)));
        }
    }

    // Patrolling birds (a moving hazard) — tagged "-" like the static ones.
    if let Some(sp) = scamper::sprite::get("swooper") {
        let an = sp.anim("walk");
        let n = an.frames.len().max(1);
        let fi = (sim.clock() / (NS_PER_SEC / an.fps.max(1) as u64)) as usize % n;
        let frame = &an.frames[fi];
        for b in birds {
            if b.pos.x < cam_x - TILE || b.pos.x > cam_x + fb_w as f64 + TILE {
                continue;
            }
            let lines: Vec<String> = frame.iter().map(|s| s.to_string()).collect();
            let fw = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
            let (mw, mh) = (fw * cpw, lines.len() as f64 * cph);
            let lx = sx(b.pos.x + b.w / 2.0) - mw / 2.0;
            let ly = sy(b.pos.y + b.h) - mh;
            let cx = lx + mw / 2.0;
            sprites.push((lines, lx, ly, sp.palette));
            let bar = bigtext("-");
            let bw = bar[0].chars().count() as f64 * cpw;
            sprites.push((bar, cx - bw / 2.0, ly - 6.0 * cph, minus_rgb as fn(char) -> (u8, u8, u8)));
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

    // Transient effects (jump puff / landing dust from Sim::step, plus our hit/heal
    // bursts) and floating word-pops, world-anchored on the tick clock.
    let fxr = sim.fx.render(sim.clock());
    let words = sim.fx.render_words(sim.clock());

    if backend.draws_overlay() {
        let mut overlays: Vec<Overlay> = sprites
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
        let fx_lines: Vec<Vec<String>> = fxr.iter().map(|(frame, ..)| frame.iter().map(|s| s.to_string()).collect()).collect();
        for ((frame, tint, z, fxx, fxy), lns) in fxr.iter().zip(fx_lines.iter()) {
            let w = frame.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
            overlays.push(Overlay { lines: lns, col: ((sx(*fxx) - w * cpw / 2.0) / cpw).round() as i32, row: (sy(*fxy) / cph).round() as i32, tint: Some(*tint), palette: None, z: 1000 + z });
        }
        let word_lines: Vec<Vec<String>> = words.iter().map(|(t, ..)| bigtext(t)).collect();
        for ((_t, tint, z, wx, wy), lns) in words.iter().zip(word_lines.iter()) {
            let wcells = lns.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
            overlays.push(Overlay {
                lines: lns,
                col: ((sx(*wx) - wcells * cpw / 2.0) / cpw).round() as i32,
                row: ((sy(*wy) - 5.0 * cph) / cph).round() as i32,
                tint: Some(*tint),
                palette: None,
                z: 2000 + z,
            });
        }
        backend.present(out, fb, cols, rows, full_redraw, &overlays);
    } else {
        for (lns, lx, ly, p) in &sprites {
            draw_sprite_pixels(fb, lns, *lx, *ly, cpw, cph, *p);
        }
        for &(frame, tint, _z, fxx, fxy) in &fxr {
            draw_effect_pixels(fb, frame, tint, sx(fxx), sy(fxy), cpw, cph);
        }
        // Words spelled as 3x5 pixel-font art, so they're legible as letter shapes
        // (a single glyph per letter would just be an unreadable block here).
        for &(text, tint, _z, wx, wy) in &words {
            let art = bigtext(text);
            let refs: Vec<&str> = art.iter().map(|s| s.as_str()).collect();
            draw_effect_pixels(fb, &refs, tint, sx(wx), sy(wy) - 5.0 * cph, cpw, cph);
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
    draw_hud(score, mult, gen::run_speed(difficulty, sim.player.pos.x), fidelity, cols, rows + 1);
}

/// A single status row (score, pristine multiplier, speed, fidelity) full width.
fn draw_hud(score: u32, mult: f64, speed: f64, fidelity: u8, cols: u16, status_row: u16) {
    let text = format!("  ⚡ {score}   ×{:.1}   {:.0} px/s   fidelity {}  ", mult, speed, fidelity_pips(fidelity));
    let text: String = text.chars().take(cols as usize).collect();
    let mut o = std::io::stdout().lock();
    let _ = write!(o, "\x1b[{};1H\x1b[2K\x1b[7m{text}\x1b[0m", status_row);
    let _ = o.flush();
}

/// A character's 3×5 pixel-font rows ('#' on, ' ' off), upper-cased. Unknown glyphs
/// fall back to a solid box so a word never silently drops a letter.
fn glyph5(c: char) -> [&'static str; 5] {
    match c.to_ascii_uppercase() {
        'A' => ["###", "# #", "###", "# #", "# #"],
        'B' => ["## ", "# #", "## ", "# #", "## "],
        'C' => ["###", "#  ", "#  ", "#  ", "###"],
        'D' => ["## ", "# #", "# #", "# #", "## "],
        'E' => ["###", "#  ", "## ", "#  ", "###"],
        'F' => ["###", "#  ", "## ", "#  ", "#  "],
        'G' => ["###", "#  ", "# #", "# #", "###"],
        'H' => ["# #", "# #", "###", "# #", "# #"],
        'I' => ["###", " # ", " # ", " # ", "###"],
        'J' => ["  #", "  #", "  #", "# #", "###"],
        'K' => ["# #", "# #", "## ", "# #", "# #"],
        'L' => ["#  ", "#  ", "#  ", "#  ", "###"],
        'M' => ["# #", "###", "###", "# #", "# #"],
        'N' => ["# #", "###", "###", "###", "# #"],
        'O' => ["###", "# #", "# #", "# #", "###"],
        'P' => ["###", "# #", "###", "#  ", "#  "],
        'Q' => ["###", "# #", "# #", "###", "  #"],
        'R' => ["## ", "# #", "## ", "# #", "# #"],
        'S' => ["###", "#  ", "###", "  #", "###"],
        'T' => ["###", " # ", " # ", " # ", " # "],
        'U' => ["# #", "# #", "# #", "# #", "###"],
        'V' => ["# #", "# #", "# #", "# #", " # "],
        'W' => ["# #", "# #", "###", "###", "# #"],
        'X' => ["# #", "# #", " # ", "# #", "# #"],
        'Y' => ["# #", "# #", " # ", " # ", " # "],
        'Z' => ["###", "  #", " # ", "#  ", "###"],
        '0' => ["###", "# #", "# #", "# #", "###"],
        '1' => [" # ", "## ", " # ", " # ", "###"],
        '2' => ["###", "  #", "###", "#  ", "###"],
        '3' => ["###", "  #", "###", "  #", "###"],
        '4' => ["# #", "# #", "###", "  #", "  #"],
        '5' => ["###", "#  ", "###", "  #", "###"],
        '6' => ["###", "#  ", "###", "# #", "###"],
        '7' => ["###", "  #", " # ", " # ", " # "],
        '8' => ["###", "# #", "###", "# #", "###"],
        '9' => ["###", "# #", "###", "  #", "###"],
        '!' => [" # ", " # ", " # ", "   ", " # "],
        '✚' => ["   ", " # ", "###", " # ", "   "], // medical cross — "this is good"
        '-' => ["   ", "   ", "###", "   ", "   "],
        ' ' => ["   ", "   ", "   ", "   ", "   "],
        _ => ["###", "###", "###", "###", "###"],
    }
}

/// Lay a string out as 5 rows of 3×5 pixel-font art (one space between letters).
fn bigtext(s: &str) -> Vec<String> {
    let glyphs: Vec<[&str; 5]> = s.chars().map(glyph5).collect();
    (0..5).map(|r| glyphs.iter().map(|g| g[r]).collect::<Vec<_>>().join(" ")).collect()
}

/// Rasterize an effect clip / word into the framebuffer (pixel tiers): each glyph a
/// cell-sized block in the effect tint. `ax`/`ay` = clip anchor (center-x, top-y).
fn draw_effect_pixels(fb: &mut Framebuffer, frame: &[&str], tint: (u8, u8, u8), ax: f64, ay: f64, cpw: f64, cph: f64) {
    let w_cells = frame.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let left = ax - w_cells as f64 * cpw / 2.0;
    let col = Rgba::rgb(tint.0, tint.1, tint.2);
    for (gr, line) in frame.iter().enumerate() {
        for (gc, ch) in line.chars().enumerate() {
            if ch != ' ' {
                let x0 = (left + gc as f64 * cpw).floor() as i32;
                let x1 = (left + (gc as f64 + 1.0) * cpw).floor() as i32;
                let y0 = (ay + gr as f64 * cph).floor() as i32;
                let y1 = (ay + (gr as f64 + 1.0) * cph).floor() as i32;
                fb.fill_rect(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1), col);
            }
        }
    }
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
        let mut birds = build_birds(&course);
        let mut score = 0.0f64;
        let mut pristine = 0u32;
        for tick in 0..6000u32 {
            let jp = jump(&sim, &course);
            let px0 = sim.player.pos.x;
            match advance(&mut sim, &course, &mut treats, &mut birds, difficulty, base_fp, jp, jp, false, death_y, course_end_px, &mut invuln) {
                Tick::Ended(EndReason::Fell) => return (Outcome::Fell { score: score as u32 }, tick),
                Tick::Ended(EndReason::Maxed) => return (Outcome::Maxed { score: score as u32 }, tick),
                Tick::Hit => {
                    let (nf, dead) = hit_result(fidelity);
                    fidelity = nf;
                    pristine = 0;
                    if dead {
                        return (Outcome::Downed { score: score as u32 }, tick);
                    }
                }
                Tick::Heal => fidelity = (fidelity + 1).min(4),
                Tick::Continue => {}
            }
            if fidelity >= 4 {
                pristine += 1;
            } else {
                pristine = 0;
            }
            score += ((sim.player.pos.x - px0).max(0.0) / TILE) * pristine_mult(pristine);
        }
        (Outcome::Quit { score: score as u32 }, 6000)
    }

    #[test]
    fn spawns_on_a_solid_roof_and_auto_runs() {
        // With NO jumping, the runner clears some flat opening roof, then falls at the
        // first gap (the opening building has no hazard) — proving spawn-on-solid,
        // auto-run, and gap-death detection.
        let (outcome, _) = simulate(7, Difficulty::Standard, 14, |_, _| false);
        match outcome {
            Outcome::Fell { score } => {
                assert!(score >= 3, "should clear some opening roof first, got {score}");
                assert!(score < 80, "without jumping it can't get far, got {score}");
            }
            other => panic!("expected a fall without jumping, got {}", other.score()),
        }
    }

    #[test]
    fn bigtext_spells_in_five_rows() {
        let art = bigtext("HI");
        assert_eq!(art.len(), 5, "3x5 font → 5 rows");
        assert_eq!(art[0].chars().count(), 7, "two glyphs + a space = 7 cols");
        assert!(art.iter().any(|r| r.contains('#')), "has ink");
        // The medical cross is a recognizable plus (ink on the middle row).
        let cross = bigtext("✚");
        assert_eq!(cross[2], "###");
    }

    #[test]
    fn time_of_day_shifts_the_sky() {
        let base = art::palette(art::Theme::Rooftop);
        let night = time_palette(base, 0.0).sky;
        let day = time_palette(base, 2.0 * PHASE_LEN_PX).sky;
        assert_ne!(night, day, "sky changes from night to day");
        assert_eq!(phase_name(0), "NIGHT");
        assert_eq!(phase_name(2), "DAY");
        assert_eq!(phase_name(5), "DAWN"); // wraps (5 % 4 == 1)
    }

    #[test]
    fn pristine_multiplier_grows_then_caps() {
        assert!((pristine_mult(0) - 1.0).abs() < 1e-9, "starts at 1x");
        assert!(pristine_mult(300) > 1.4 && pristine_mult(300) < 1.6, "climbs");
        assert!((pristine_mult(100_000) - 3.0).abs() < 1e-9, "caps at 3x");
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
        let mut birds: Vec<Mob> = Vec::new();
        let r = advance(&mut sim, &course, &mut treats, &mut birds, Difficulty::Standard, gen::base_feel(), false, false, false, 24.0 * TILE, 1.0e9, &mut invuln);
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
        let mut birds: Vec<Mob> = Vec::new();
        let mut last = sim.player.pos.x;
        for _ in 0..30 {
            advance(&mut sim, &course, &mut treats, &mut birds, Difficulty::Cruise, base_fp, false, false, false, 14.0 * TILE, 4000.0 * TILE, &mut invuln);
            assert!(sim.player.pos.x >= last - 0.001, "x went backwards: {} < {}", sim.player.pos.x, last);
            last = sim.player.pos.x;
        }
    }
}

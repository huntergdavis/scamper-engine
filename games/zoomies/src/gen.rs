//! Procedural rooftop course generation.
//!
//! One long `TileMap` is built up-front from a seed: a left-to-right sequence of
//! building segments (solid columns from a roof row down to the map floor) separated
//! by gaps (empty full-height slots — fall in and you die). It is deterministic
//! (seeded xorshift, no wall-clock) so a seed reproduces a course exactly.
//!
//! Fairness is anchored to the *real* physics: [`jump_envelope`] simulates an actual
//! `Player` jump at a given run speed and measures how far it travels and how high it
//! rises. The generator only places a gap/step the player can clear at the speed it
//! will have arrived there (speed ramps with distance). [`bounds`] turns that envelope
//! into conservative tile budgets; the generator and the fairness test share it, so
//! "fair" can't drift from the physics.
#![allow(dead_code)] // wired into the gameplay loop in the next step

use crate::Difficulty;
use scamper::player::{FeelParams, Player};
use scamper::sim::SIM_DT;
use scamper::world::{TileMap, TILE};

/// The movement feel Zoomies runs on (also what gameplay uses, with `max_run` ramped
/// to the current speed). Default for now; tuned later if needed.
pub fn base_feel() -> FeelParams {
    FeelParams::default()
}

/// Deterministic xorshift64 — seeded RNG with no wall-clock, so courses replay.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1) // avoid the all-zero state
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Inclusive integer in `[lo, hi]`.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % ((hi - lo + 1) as u64)) as i32
    }
}

/// One stretch of the course.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    /// A solid building: `roof` is the top row (smaller = higher), `width` in tiles.
    Building { roof: i32, width: i32 },
    /// An empty gap `width` tiles wide — falling in is fatal.
    Gap { width: i32 },
}

/// A static rooftop hazard (a spiky vent): touch it and you lose a fidelity tier.
/// Placed mid-building, clear of the edges, low enough to clear with a jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A generated course: the tilemap plus the segment list (for gameplay + the
/// fairness test), the rooftop hazards, and the spawn point on the first roof.
pub struct Course {
    pub map: TileMap,
    pub segs: Vec<Seg>,
    pub obstacles: Vec<Obstacle>,
    pub spawn: (f64, f64),
    pub rows: i32,
    pub width_tiles: i32,
}

/// Difficulty knobs for generation + the speed ramp.
struct Tuning {
    start_speed: f64,  // px/s at x=0
    max_speed: f64,    // px/s cap
    ramp_px: f64,      // distance (px) over which speed climbs from start to max
    bw: (i32, i32),    // building width range (tiles)
    roof: (i32, i32),  // roof-row range (min=highest, max=lowest)
    max_down: i32,     // largest drop (tiles) to a lower roof
    haz_pct: i32,      // chance (0..100) a wide-enough building carries a hazard
}

fn tuning(d: Difficulty, rows: i32) -> Tuning {
    let (start, max, ramp, bw, haz) = match d {
        Difficulty::Cruise => (170.0, 300.0, 5000.0, (6, 11), 25),
        Difficulty::Standard => (210.0, 410.0, 4000.0, (4, 9), 45),
        Difficulty::Frantic => (255.0, 540.0, 3200.0, (3, 7), 70),
    };
    Tuning {
        start_speed: start,
        max_speed: max,
        ramp_px: ramp,
        bw,
        haz_pct: haz,
        // Roofs occupy the upper-middle band; never so low that there's no fall room.
        roof: ((rows as f64 * 0.28) as i32, (rows as f64 * 0.62) as i32),
        max_down: 4,
    }
}

/// Run speed (px/s) at world x (px): a linear ramp from `start` to `max`.
fn speed_at(t: &Tuning, x_px: f64) -> f64 {
    let f = (x_px / t.ramp_px).clamp(0.0, 1.0);
    t.start_speed + (t.max_speed - t.start_speed) * f
}

/// The auto-run speed (px/s) at world x for a difficulty — what the gameplay loop
/// ramps `max_run` to. (Speed is independent of row count.)
pub fn run_speed(difficulty: Difficulty, x_px: f64) -> f64 {
    speed_at(&tuning(difficulty, 24), x_px)
}

/// Safety margin: the player is only asked to clear 80% of what the physics allow,
/// so imperfect timing still makes it.
const SAFETY: f64 = 0.8;

/// Simulate one real jump at run speed `v` (px/s) on flat ground, returning
/// `(horizontal_reach_px, peak_height_px)` — full-hold (max height), single jump
/// (no double-jump assumed, so the budget is conservative).
pub fn jump_envelope(fp: &FeelParams, v: f64) -> (f64, f64) {
    let (cols, rows, floor_row) = (60usize, 24usize, 18usize);
    let mut map = TileMap::new(cols, rows);
    for tx in 0..cols {
        for ty in floor_row..rows {
            map.set(tx, ty, true);
        }
    }
    let mut fp = *fp;
    fp.max_run = v.max(1.0);
    let base_y = floor_row as f64 * TILE - 16.0;
    let mut p = Player::new(4.0 * TILE, base_y);
    for _ in 0..10 {
        p.step(&map, SIM_DT, 1.0, false, false, false, &fp); // settle onto the roof
    }
    p.vel.x = v; // arrive at the jump already at run speed
    let (takeoff_x, takeoff_y) = (p.pos.x, p.pos.y);
    let (mut reach, mut peak) = (0.0_f64, 0.0_f64);
    let mut airborne = false;
    for t in 0..240 {
        p.step(&map, SIM_DT, 1.0, t == 0, true, false, &fp);
        peak = peak.max(takeoff_y - p.pos.y);
        if !p.grounded {
            airborne = true;
        }
        if airborne && p.grounded {
            reach = p.pos.x - takeoff_x;
            break;
        }
    }
    (reach.max(0.0), peak.max(0.0))
}

/// Conservative tile budgets at run speed `v`: `(max_gap_tiles, max_climb_tiles)`.
pub fn bounds(fp: &FeelParams, v: f64) -> (i32, i32) {
    let (reach, peak) = jump_envelope(fp, v);
    let gap = ((SAFETY * reach) / TILE).floor() as i32;
    let climb = ((SAFETY * peak) / TILE).floor() as i32;
    (gap.max(1), climb.max(1))
}

/// Generate a course of about `width_tiles` columns and `rows` rows for `difficulty`.
pub fn generate(seed: u64, difficulty: Difficulty, width_tiles: i32, rows: i32) -> Course {
    let t = tuning(difficulty, rows);
    let fp = base_feel();
    let mut rng = Rng::new(seed);
    let mut map = TileMap::new(width_tiles as usize, rows as usize);
    let mut segs: Vec<Seg> = Vec::new();
    let mut obstacles: Vec<Obstacle> = Vec::new();

    // First building: a generous flat run to stand on at spawn.
    let mut cur_roof = (t.roof.0 + t.roof.1) / 2;
    let first_w = t.bw.1.max(6);
    let mut x = 0;
    fill_building(&mut map, x, first_w, cur_roof, rows);
    segs.push(Seg::Building { roof: cur_roof, width: first_w });
    x += first_w;
    let spawn = (2.0 * TILE, cur_roof as f64 * TILE - 16.0);

    while x < width_tiles {
        // A gap, then the next building — sized so the jump from `cur_roof` lands.
        let v = speed_at(&t, x as f64 * TILE);
        let (max_gap, max_climb) = bounds(&fp, v);

        // Pick the next roof first (so the gap budget can account for any climb).
        // Cap the climb so a minimum 1-tile gap still fits the combined budget
        // (each climbed tile costs ~2 of horizontal reach): up ≤ (max_gap-1)/2.
        let up_room = max_climb.min((max_gap - 1) / 2).min(cur_roof - t.roof.0).max(0);
        let down_room = t.max_down.min(t.roof.1 - cur_roof);
        let next_roof = cur_roof + rng.range(-up_room, down_room.max(0));
        let up = (cur_roof - next_roof).max(0); // tiles climbed (0 if flat/down)

        // Combined budget: each tile climbed costs ~2 tiles of horizontal reach.
        let gap_cap = (max_gap - 2 * up).max(1);
        let gap_w = rng.range(1, gap_cap).min(gap_cap);
        segs.push(Seg::Gap { width: gap_w });
        x += gap_w;
        if x >= width_tiles {
            break;
        }

        let bw = rng.range(t.bw.0, t.bw.1).min(width_tiles - x).max(1);
        fill_building(&mut map, x, bw, next_roof, rows);
        segs.push(Seg::Building { roof: next_roof, width: bw });

        // Maybe a rooftop hazard, mid-building (≥2 tiles from each edge) so it never
        // stacks with an edge jump. One tile tall — clearable with a hop.
        if bw >= 6 && rng.range(0, 99) < t.haz_pct {
            let col = x + rng.range(2, bw - 3);
            obstacles.push(Obstacle {
                x: col as f64 * TILE + 2.0,
                y: next_roof as f64 * TILE - 14.0,
                w: 12.0,
                h: 14.0,
            });
        }
        x += bw;
        cur_roof = next_roof;
    }

    map.spawn = spawn;
    Course { map, segs, obstacles, spawn, rows, width_tiles }
}

/// Fill a building's solid columns: from `roof` down to the map floor, `width` tiles
/// wide starting at column `x0`.
fn fill_building(map: &mut TileMap, x0: i32, width: i32, roof: i32, rows: i32) {
    for tx in x0..(x0 + width) {
        for ty in roof..rows {
            map.set(tx as usize, ty as usize, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_envelope_is_sane_and_pinned() {
        // Hand-checked guard: at the default top speed the player clears a few tiles
        // and rises a bit over a tile. If physics or feel drift, this fails loudly so
        // the calibration can't silently go wrong.
        let fp = base_feel();
        let (reach, peak) = jump_envelope(&fp, 215.0);
        assert!(reach > 40.0 && reach < 260.0, "reach {reach} px out of expected band");
        assert!(peak > 16.0 && peak < 160.0, "peak {peak} px out of expected band");
    }

    #[test]
    fn same_seed_same_course() {
        let a = generate(12345, Difficulty::Standard, 600, 24);
        let b = generate(12345, Difficulty::Standard, 600, 24);
        assert_eq!(a.segs, b.segs, "identical seed must reproduce the segment list");
        assert_eq!(a.map.solid, b.map.solid, "...and the exact tilemap");
        // A different seed should (almost surely) differ.
        let c = generate(999, Difficulty::Standard, 600, 24);
        assert_ne!(a.segs, c.segs);
    }

    #[test]
    fn obstacles_sit_on_solid_roofs_and_are_low() {
        let fp = base_feel();
        for d in Difficulty::ALL {
            let course = generate(3, d, 1500, 24);
            let min_climb_px = bounds(&fp, run_speed(d, 0.0)).1 as f64 * TILE;
            for o in &course.obstacles {
                assert!(o.h <= TILE, "{d:?}: hazard taller than a tile: {}", o.h);
                assert!(o.h <= min_climb_px, "{d:?}: hazard {o:?} not clearable (climb {min_climb_px}px)");
                // The tile directly under the hazard's base must be solid roof.
                let tx = (o.x / TILE) as i32;
                let ty = ((o.y + o.h) / TILE) as i32;
                assert!(course.map.is_solid(tx, ty), "{d:?}: hazard {o:?} not resting on a roof");
            }
        }
    }

    #[test]
    fn every_gap_and_step_is_clearable() {
        // The fairness invariant, checked against the SAME physics-derived budgets the
        // generator used: across every difficulty, each gap fits the jump reach at the
        // speed reached there, each up-step fits the climb, and the combined
        // gap+2·climb budget holds.
        let fp = base_feel();
        for d in Difficulty::ALL {
            let course = generate(7, d, 1500, 24);
            let t = tuning(d, 24);
            let mut x_tiles = 0i32;
            let mut prev_roof: Option<i32> = None;
            let mut last_gap: Option<i32> = None;
            for seg in &course.segs {
                match *seg {
                    Seg::Gap { width } => {
                        let v = speed_at(&t, x_tiles as f64 * TILE);
                        let (max_gap, _) = bounds(&fp, v);
                        assert!(width <= max_gap, "{d:?}: gap {width} > reach {max_gap} at x={x_tiles}");
                        last_gap = Some(width);
                        x_tiles += width;
                    }
                    Seg::Building { roof, width } => {
                        if let (Some(pr), Some(g)) = (prev_roof, last_gap) {
                            let v = speed_at(&t, (x_tiles - g) as f64 * TILE);
                            let (max_gap, max_climb) = bounds(&fp, v);
                            let up = (pr - roof).max(0);
                            assert!(up <= max_climb, "{d:?}: up-step {up} > climb {max_climb}");
                            assert!(g + 2 * up <= max_gap, "{d:?}: combined {g}+2*{up} > {max_gap}");
                        }
                        prev_roof = Some(roof);
                        last_gap = None;
                        x_tiles += width;
                    }
                }
            }
        }
    }
}

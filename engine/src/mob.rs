//! Mobs — a generic tile-colliding actor for creatures and items, plus the AABB
//! collision primitives the game uses to resolve pounces and pickups
//! (CAMPAIGN_PLAN.md §3, §7). This is an **engine primitive**: it knows how a box
//! walks, falls, and bumps walls/ledges, but nothing about *which* creature it is
//! or *what* a collision means — the game owns that.
//!
//! Behavior is a small deterministic FSM advanced one tick at a time, reading only
//! its own state and the tilemap (no wall-clock, no RNG), so a recorded run
//! replays a mob's path exactly — the same guarantee the player sim gives.

use crate::math::{vec2, Vec2};
use crate::world::{TileMap, TILE};

/// How a mob walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    /// Walk forward, reverse at walls, and stroll off ledges (Goomba-like).
    Wander,
    /// Walk forward, reverse at walls *and* at ledges — never falls off.
    Careful,
    /// Inert (items / power-ups): gravity only, no walking.
    Still,
    /// Fly horizontally at a fixed height — no gravity; reverses at walls. Used by
    /// air creatures and (with a lifetime the game manages) projectiles.
    Fly,
    /// Bob up and down around the home position (rise-and-lower) — pipe plants.
    Bob,
    /// Like [`Wander`](Gait::Wander), but springs into a periodic hop whenever it's
    /// on the ground — a bouncing critter (frog / spring).
    Hop,
    /// Cruise horizontally (no gravity) while weaving up and down in a sine wave
    /// around the home height — a swooping flyer (moth / bat).
    Swoop,
    /// Drift in the facing direction ignoring *all* collision and gravity — a
    /// phasing mover (a ghost). The game steers `facing`/`speed`/`pos.y` directly.
    Phase,
    /// Free projectile: gravity-driven arc, moves by its own velocity, stops dead
    /// on any solid (sets `blocked`). Used for thrown sticks.
    Ballistic,
}

/// Per-tick physics constants, tuned for the 16px tile world.
const GRAVITY: f64 = 0.30; // px/tick²
const MAX_FALL: f64 = 6.0; // px/tick

/// A box that falls, walks, and collides with solid tiles.
#[derive(Clone, Debug)]
pub struct Mob {
    pub pos: Vec2, // top-left of the AABB
    pub vel: Vec2,
    pub w: f64,
    pub h: f64,
    pub facing: i8, // -1 left, +1 right
    pub speed: f64, // px/tick when walking
    pub gait: Gait,
    pub alive: bool,
    pub age: u32,      // ticks lived (for timers / oscillation)
    pub blocked: bool, // hit a wall on the last horizontal move (projectiles die on this)
    pub home_y: f64,   // anchor for the Bob gait (set to the spawn y)
}

/// Bob (pipe-plant) oscillation: rise this many px and back over this many ticks.
const BOB_AMP: f64 = 26.0;
const BOB_PERIOD: f64 = 150.0;

/// Hop gait: spring up with this velocity every `HOP_PERIOD` ticks while grounded.
const HOP_VY: f64 = 3.6; // px/tick (≈1.3-tile arc under GRAVITY)
const HOP_PERIOD: u32 = 84;

/// Swoop gait: weave this many px above/below the home height over this period.
const SWOOP_AMP: f64 = 22.0;
const SWOOP_PERIOD: f64 = 70.0;

impl Mob {
    pub fn new(x: f64, y: f64, w: f64, h: f64, facing: i8, speed: f64, gait: Gait) -> Self {
        Mob { pos: vec2(x, y), vel: vec2(0.0, 0.0), w, h, facing, speed, gait, alive: true, age: 0, blocked: false, home_y: y }
    }

    /// Advance one tick against the solid tilemap.
    pub fn step(&mut self, map: &TileMap) {
        if !self.alive {
            return;
        }
        self.age = self.age.wrapping_add(1);
        self.blocked = false;

        // Flyers ignore gravity: cruise horizontally, reverse at walls.
        if self.gait == Gait::Fly {
            let dir = if self.facing >= 0 { 1.0 } else { -1.0 };
            if self.move_axis(map, dir * self.speed, 0.0) {
                self.facing = -self.facing;
                self.blocked = true;
            }
            return;
        }

        // Phase: translate in the facing direction through everything (a ghost).
        // No collision, no gravity — the game owns facing/speed/vertical drift.
        if self.gait == Gait::Phase {
            let dir = if self.facing >= 0 { 1.0 } else { -1.0 };
            self.pos.x += dir * self.speed;
            return;
        }

        // Swoop: cruise horizontally (reverse at walls) while weaving on a sine.
        if self.gait == Gait::Swoop {
            let dir = if self.facing >= 0 { 1.0 } else { -1.0 };
            if self.move_axis(map, dir * self.speed, 0.0) {
                self.facing = -self.facing;
                self.blocked = true;
            }
            let w = std::f64::consts::TAU / SWOOP_PERIOD;
            self.pos.y = self.home_y + SWOOP_AMP * (self.age as f64 * w).sin();
            return;
        }

        // Bob: rise from the home position and lower again (pipe plants). Set the
        // position directly — a smooth raised-cosine so it eases at both ends.
        if self.gait == Gait::Bob {
            let w = std::f64::consts::TAU / BOB_PERIOD;
            self.pos.y = self.home_y - BOB_AMP * 0.5 * (1.0 - (self.age as f64 * w).cos());
            return;
        }

        // Ballistic: a free projectile — gravity-driven arc, dies on any solid.
        if self.gait == Gait::Ballistic {
            self.vel.y = (self.vel.y + GRAVITY).min(MAX_FALL);
            if self.move_axis(map, self.vel.x, 0.0) {
                self.blocked = true;
            }
            if self.move_axis(map, 0.0, self.vel.y) {
                self.blocked = true;
            }
            return;
        }

        // Grounded gaits: gravity always applies (even Still items settle).
        self.vel.y = (self.vel.y + GRAVITY).min(MAX_FALL);
        // Hop: spring off the ground on a fixed cadence (a bouncing critter).
        if self.gait == Gait::Hop && self.age % HOP_PERIOD == 0 && self.on_ground(map) {
            self.vel.y = -HOP_VY;
        }
        if self.gait != Gait::Still && self.speed != 0.0 {
            let dir = if self.facing >= 0 { 1.0 } else { -1.0 };
            // Careful gait: turn around rather than step off a ledge.
            if self.gait == Gait::Careful && self.on_ground(map) && !self.ground_ahead(map, dir) {
                self.facing = -self.facing;
            }
            let dir = if self.facing >= 0 { 1.0 } else { -1.0 };
            if self.move_axis(map, dir * self.speed, 0.0) {
                self.facing = -self.facing; // bumped a wall → turn back
                self.blocked = true;
            }
        }
        self.move_axis(map, 0.0, self.vel.y);
    }

    /// Resting on (or 1px above) solid ground (incl. one-way platform tops)?
    pub fn on_ground(&self, map: &TileMap) -> bool {
        map.overlaps(self.pos.x, self.pos.y + 1.0, self.w, self.h)
            || map.on_oneway(self.pos.x, self.w, self.pos.y + self.h)
    }

    /// Is there solid ground just beyond the leading foot (for ledge-careful gait)?
    fn ground_ahead(&self, map: &TileMap, dir: f64) -> bool {
        let foot_x = if dir > 0.0 { self.pos.x + self.w + 1.0 } else { self.pos.x - 1.0 };
        let foot_y = self.pos.y + self.h + 1.0;
        map.is_solid((foot_x / TILE).floor() as i32, (foot_y / TILE).floor() as i32)
    }

    /// Move along one axis in ≤1px substeps; returns true if blocked by a solid.
    /// Zeros the vertical velocity on a vertical hit (landing / head-bump).
    fn move_axis(&mut self, map: &TileMap, dx: f64, dy: f64) -> bool {
        let dist = dx.abs() + dy.abs();
        if dist == 0.0 {
            return false;
        }
        let sx = if dx > 0.0 { 1.0 } else if dx < 0.0 { -1.0 } else { 0.0 };
        let sy = if dy > 0.0 { 1.0 } else if dy < 0.0 { -1.0 } else { 0.0 };
        let mut rem = dist;
        while rem > 0.0 {
            let s = rem.min(1.0);
            let nx = self.pos.x + sx * s;
            let ny = self.pos.y + sy * s;
            let blocked = map.overlaps(nx, ny, self.w, self.h)
                || (dy > 0.0 && map.lands_on_oneway(nx, self.w, self.pos.y + self.h, ny + self.h));
            if blocked {
                if dy != 0.0 {
                    self.vel.y = 0.0;
                }
                return true;
            }
            self.pos.x = nx;
            self.pos.y = ny;
            rem -= s;
        }
        false
    }
}

/// Do two AABBs overlap?
#[allow(clippy::too_many_arguments)]
pub fn aabb_overlap(ax: f64, ay: f64, aw: f64, ah: f64, bx: f64, by: f64, bw: f64, bh: f64) -> bool {
    ax < bx + bw && ax + aw > bx && ay < by + bh && ay + ah > by
}

/// Is box `a` landing on top of box `b` — descending, overlapping, and with its
/// feet in `b`'s upper band? This is the "pounce / stomp" test (game decides the
/// consequence). `avy` is `a`'s vertical velocity (down is positive).
#[allow(clippy::too_many_arguments)]
pub fn stomp(ax: f64, ay: f64, aw: f64, ah: f64, avy: f64, bx: f64, by: f64, bw: f64, bh: f64) -> bool {
    aabb_overlap(ax, ay, aw, ah, bx, by, bw, bh) && avy > 0.0 && (ay + ah) <= by + bh * 0.6
}

#[cfg(test)]
mod tests {
    use super::*;

    // Full floor at row 4 with a wall pillar at column x=8 (rows 0..3).
    fn flat() -> TileMap {
        TileMap::from_ascii(&[
            "........#...",
            "........#...",
            "........#...",
            "........#...",
            "############",
        ])
    }

    // Floor spans x=0..5, then a gap (x=6..11) — a ledge to fall off / turn at.
    fn ledge() -> TileMap {
        TileMap::from_ascii(&["............", "............", "............", "............", "######......"])
    }

    fn settle(m: &mut Mob, map: &TileMap, ticks: usize) {
        for _ in 0..ticks {
            m.step(map);
        }
    }

    #[test]
    fn falls_and_lands_on_the_floor() {
        let map = flat();
        let mut m = Mob::new(1.0 * TILE, 0.0, 12.0, 14.0, 1, 0.0, Gait::Still);
        settle(&mut m, &map, 60);
        assert!(m.on_ground(&map), "should rest on the floor");
        assert!((m.pos.y + m.h - 4.0 * TILE).abs() < 1.5, "feet at the floor top");
    }

    #[test]
    fn wander_reverses_at_a_wall() {
        let map = flat();
        // start on the floor, left of the wall at x=8, walking right
        let mut m = Mob::new(2.0 * TILE, 3.0 * TILE, 12.0, 14.0, 1, 1.0, Gait::Wander);
        settle(&mut m, &map, 200);
        assert_eq!(m.facing, -1, "should have turned around at the wall");
    }

    #[test]
    fn careful_turns_at_a_ledge_but_wander_walks_off() {
        let map = ledge();
        let mut careful = Mob::new(2.0 * TILE, 3.0 * TILE, 12.0, 14.0, 1, 1.0, Gait::Careful);
        settle(&mut careful, &map, 120);
        assert!(careful.on_ground(&map), "careful mob never leaves the ledge");
        assert!(careful.pos.y < 4.5 * TILE, "careful mob did not fall into the gap");

        let mut wanderer = Mob::new(2.0 * TILE, 3.0 * TILE, 12.0, 14.0, 1, 1.0, Gait::Wander);
        settle(&mut wanderer, &map, 120);
        assert!(wanderer.pos.y > 5.0 * TILE, "wanderer walked off and fell into the gap");
    }

    #[test]
    fn hop_springs_off_the_ground_periodically() {
        let map = flat();
        let mut m = Mob::new(2.0 * TILE, 3.0 * TILE, 12.0, 14.0, 1, 0.8, Gait::Hop);
        settle(&mut m, &map, 10); // let it land
        let resting = m.pos.y;
        // Within one hop period it should have left the ground (risen above rest).
        let mut peaked = resting;
        for _ in 0..super::HOP_PERIOD + 4 {
            m.step(&map);
            peaked = peaked.min(m.pos.y);
        }
        assert!(peaked < resting - 6.0, "hop should lift the critter off the floor (rest {resting}, peak {peaked})");
    }

    #[test]
    fn swoop_weaves_around_its_home_height_and_moves_sideways() {
        let map = flat();
        let start_x = 2.0 * TILE;
        let mut m = Mob::new(start_x, 2.0 * TILE, 12.0, 12.0, 1, 1.0, Gait::Swoop);
        let home = m.home_y;
        let (mut lo, mut hi) = (home, home);
        for _ in 0..80 {
            m.step(&map);
            lo = lo.min(m.pos.y);
            hi = hi.max(m.pos.y);
        }
        assert!(hi - lo > 20.0, "swoop should weave vertically (span {})", hi - lo);
        assert!(m.pos.x > start_x, "and drift sideways");
        assert!(m.pos.y > 0.0, "never falls under gravity");
    }

    #[test]
    fn phase_drifts_through_walls_without_gravity() {
        let map = flat(); // has a wall at x=8
        let start_y = 3.0 * TILE;
        let mut m = Mob::new(2.0 * TILE, start_y, 12.0, 12.0, 1, 1.0, Gait::Phase);
        for _ in 0..200 {
            m.step(&map);
        }
        assert!(m.pos.x > 8.0 * TILE, "phases straight through the wall (x={})", m.pos.x);
        assert_eq!(m.pos.y, start_y, "no gravity — holds its height");
    }

    #[test]
    fn flyer_ignores_gravity_and_reverses_at_walls() {
        let map = flat(); // wall pillar at x=8
        let mut f = Mob::new(2.0 * TILE, 1.0 * TILE, 12.0, 12.0, 1, 1.0, Gait::Fly);
        let y0 = f.pos.y;
        settle(&mut f, &map, 200);
        assert!((f.pos.y - y0).abs() < 0.01, "a flyer does not fall");
        assert_eq!(f.facing, -1, "reversed at the wall");
    }

    #[test]
    fn bob_rises_from_home_and_returns() {
        let map = flat();
        let home = 3.0 * TILE;
        let mut d = Mob::new(2.0 * TILE, home, 12.0, 14.0, 1, 0.0, Gait::Bob);
        assert!((d.pos.y - home).abs() < 0.01, "starts at home");
        // Quarter-ish into the cycle it has risen above home.
        settle(&mut d, &map, (BOB_PERIOD as usize) / 2);
        assert!(d.pos.y < home - BOB_AMP * 0.5, "rose well above home at mid-cycle");
        // A full period returns it home.
        settle(&mut d, &map, (BOB_PERIOD as usize) / 2);
        assert!((d.pos.y - home).abs() < 1.0, "back home after a full period");
    }

    #[test]
    fn ballistic_arcs_and_dies_on_a_solid() {
        let map = flat();
        let mut s = Mob::new(2.0 * TILE, 1.0 * TILE, 4.0, 4.0, 1, 0.0, Gait::Ballistic);
        s.vel = vec2(2.0, -4.0); // lobbed up and to the right
        let y0 = s.pos.y;
        s.step(&map);
        assert!(s.pos.y < y0, "rises just after launch");
        // Eventually it falls and lands on the floor → blocked.
        settle(&mut s, &map, 200);
        assert!(s.blocked, "a ballistic projectile dies when it hits a solid");
    }

    #[test]
    fn lands_on_oneway_from_above_but_passes_through_from_below() {
        // one-way platform row at y=2 ('='), solid floor at y=4
        let map = TileMap::from_ascii(&["....", "....", "====", "....", "####"]);

        // Falling from above → lands on the platform top (y = 2*TILE), not the floor.
        let mut faller = Mob::new(1.0 * TILE, 0.0, 12.0, 14.0, 1, 0.0, Gait::Still);
        settle(&mut faller, &map, 60);
        assert!((faller.pos.y + faller.h - 2.0 * TILE).abs() < 1.5, "rests on platform top: {}", faller.pos.y);
        assert!(faller.on_ground(&map), "grounded on the one-way platform");

        // Rising from below → passes straight through the platform.
        let mut riser = Mob::new(1.0 * TILE, 3.0 * TILE, 12.0, 14.0, 1, 0.0, Gait::Ballistic);
        riser.vel = vec2(0.0, -6.0);
        riser.step(&map);
        assert!(riser.pos.y < 3.0 * TILE - 4.0, "moving up passes through the platform");
    }

    #[test]
    fn stomp_only_from_above_while_descending() {
        // a (player) above b (creature), descending → stomp
        assert!(stomp(10.0, 10.0, 12.0, 16.0, 2.0, 10.0, 24.0, 12.0, 12.0));
        // same boxes but rising → not a stomp
        assert!(!stomp(10.0, 10.0, 12.0, 16.0, -2.0, 10.0, 24.0, 12.0, 12.0));
        // side overlap (feet well below b's top band) → not a stomp
        assert!(!stomp(10.0, 30.0, 12.0, 16.0, 2.0, 10.0, 24.0, 12.0, 12.0));
    }
}

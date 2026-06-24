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
}

impl Mob {
    pub fn new(x: f64, y: f64, w: f64, h: f64, facing: i8, speed: f64, gait: Gait) -> Self {
        Mob { pos: vec2(x, y), vel: vec2(0.0, 0.0), w, h, facing, speed, gait, alive: true }
    }

    /// Advance one tick against the solid tilemap.
    pub fn step(&mut self, map: &TileMap) {
        if !self.alive {
            return;
        }
        // Gravity always applies (even Still items settle onto the floor).
        self.vel.y = (self.vel.y + GRAVITY).min(MAX_FALL);

        if self.gait != Gait::Still && self.speed != 0.0 {
            let dir = if self.facing >= 0 { 1.0 } else { -1.0 };
            // Careful gait: turn around rather than step off a ledge.
            if self.gait == Gait::Careful && self.on_ground(map) && !self.ground_ahead(map, dir) {
                self.facing = -self.facing;
            }
            let dir = if self.facing >= 0 { 1.0 } else { -1.0 };
            if self.move_axis(map, dir * self.speed, 0.0) {
                self.facing = -self.facing; // bumped a wall → turn back
            }
        }
        self.move_axis(map, 0.0, self.vel.y);
    }

    /// Resting on (or 1px above) solid ground?
    pub fn on_ground(&self, map: &TileMap) -> bool {
        map.overlaps(self.pos.x, self.pos.y + 1.0, self.w, self.h)
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
            if map.overlaps(nx, ny, self.w, self.h) {
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
    fn stomp_only_from_above_while_descending() {
        // a (player) above b (creature), descending → stomp
        assert!(stomp(10.0, 10.0, 12.0, 16.0, 2.0, 10.0, 24.0, 12.0, 12.0));
        // same boxes but rising → not a stomp
        assert!(!stomp(10.0, 10.0, 12.0, 16.0, -2.0, 10.0, 24.0, 12.0, 12.0));
        // side overlap (feet well below b's top band) → not a stomp
        assert!(!stomp(10.0, 30.0, 12.0, 16.0, 2.0, 10.0, 24.0, 12.0, 12.0));
    }
}

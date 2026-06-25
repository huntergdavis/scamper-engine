//! Player physics, state machine, and tunable movement feel (PROJECT_PLAN.md §4.6).
//!
//! N++-flavored: dual-gravity variable jump (no velocity-cut), momentum-y accel vs
//! friction, terminal velocity, coyote time, jump buffering, double jump, wall slide
//! (downward-velocity clamp) and wall jump with a brief horizontal input lock.
//! Collision is axis-separated sub-stepped AABB vs the tile world.
//!
//! All feel constants are in px and seconds (px/s, px/s^2) so they're frame-rate
//! independent and live-tunable.

use crate::math::{vec2, Vec2};
use crate::world::TileMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Grounded,
    Airborne,
    WallSliding,
}

#[derive(Clone, Copy, Debug)]
pub struct FeelParams {
    pub gravity_rise: f64,
    pub gravity_fall: f64,
    pub max_fall: f64,
    pub run_accel: f64,
    pub air_accel: f64,
    pub ground_friction: f64,
    pub air_friction: f64,
    pub max_run: f64,
    pub jump_speed: f64,
    pub coyote_time: f64,
    pub jump_buffer: f64,
    pub max_air_jumps: i32,
    pub wall_slide_max_fall: f64,
    pub wall_jump_vx: f64,
    pub wall_jump_vy: f64,
    pub wall_jump_lock: f64,
    pub down_fast_fall: f64,
}

impl Default for FeelParams {
    fn default() -> Self {
        FeelParams {
            gravity_rise: 900.0,
            gravity_fall: 2200.0,
            max_fall: 760.0,
            run_accel: 1800.0,
            air_accel: 1500.0,
            ground_friction: 1600.0,
            air_friction: 400.0,
            max_run: 230.0,
            jump_speed: 360.0,
            coyote_time: 0.09,
            jump_buffer: 0.10,
            max_air_jumps: 1,
            wall_slide_max_fall: 130.0,
            wall_jump_vx: 245.0,
            wall_jump_vy: 360.0,
            wall_jump_lock: 0.12,
            down_fast_fall: 1400.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub pos: Vec2, // top-left of AABB
    pub vel: Vec2,
    pub w: f64,
    pub h: f64,
    pub grounded: bool,
    pub wall_dir: i32, // -1 left wall, +1 right wall, 0 none
    pub facing: i32,
    pub state: State,
    pub coyote: f64,
    pub buffer: f64,
    pub air_jumps: i32,
    pub wall_lock: f64,
    pub jumping: bool, // in a held jump rise (controls dual gravity)
    pub did_double: bool,
    pub was_engaged: bool, // was engaging a wall last frame (for rising-edge regrant)
}

const PROBE: f64 = 1.0;
const INSET: f64 = 2.0;

impl Player {
    pub fn new(x: f64, y: f64) -> Self {
        Player {
            pos: vec2(x, y),
            vel: Vec2::ZERO,
            w: 12.0,
            h: 16.0,
            grounded: false,
            wall_dir: 0,
            facing: 1,
            state: State::Airborne,
            coyote: 0.0,
            buffer: 0.0,
            air_jumps: 0,
            wall_lock: 0.0,
            jumping: false,
            did_double: false,
            was_engaged: false,
        }
    }

    fn detect_grounded(&self, map: &TileMap) -> bool {
        map.overlaps(self.pos.x + INSET, self.pos.y + self.h, self.w - 2.0 * INSET, PROBE)
            || map.on_oneway(self.pos.x + INSET, self.w - 2.0 * INSET, self.pos.y + self.h)
    }
    fn detect_wall(&self, map: &TileMap) -> i32 {
        let left = map.overlaps(self.pos.x - PROBE, self.pos.y + INSET, PROBE, self.h - 2.0 * INSET);
        let right = map.overlaps(self.pos.x + self.w, self.pos.y + INSET, PROBE, self.h - 2.0 * INSET);
        if right {
            1
        } else if left {
            -1
        } else {
            0
        }
    }

    /// Advance one fixed sim step.
    pub fn step(
        &mut self,
        map: &TileMap,
        dt: f64,
        in_x: f64,
        jump_pressed: bool,
        jump_held: bool,
        down_held: bool,
        fp: &FeelParams,
    ) {
        // --- contacts (from current rest position) ---
        self.grounded = self.detect_grounded(map);
        self.wall_dir = self.detect_wall(map);

        // --- timers ---
        if self.grounded {
            self.coyote = fp.coyote_time;
            self.air_jumps = fp.max_air_jumps;
            self.did_double = false;
            self.jumping = false;
        } else {
            self.coyote = (self.coyote - dt).max(0.0);
        }
        // Wall engagement: pressing toward the wall, or already wall-sliding last frame.
        let pressing_wall =
            (in_x > 0.0 && self.wall_dir > 0) || (in_x < 0.0 && self.wall_dir < 0);
        let engaged = self.wall_dir != 0
            && !self.grounded
            && (pressing_wall || self.state == State::WallSliding);
        // Regrant the air jump only on a FRESH engagement (rising edge) — never every
        // frame, or merely scraping a wall would grant infinite air jumps.
        if engaged && !self.was_engaged {
            self.air_jumps = fp.max_air_jumps;
            self.did_double = false;
        }
        self.was_engaged = engaged;
        self.buffer = (self.buffer - dt).max(0.0);
        self.wall_lock = (self.wall_lock - dt).max(0.0);
        if jump_pressed {
            self.buffer = fp.jump_buffer;
        }

        // --- jump resolution (buffered) ---
        if self.buffer > 0.0 {
            if self.grounded || self.coyote > 0.0 {
                self.vel.y = -fp.jump_speed;
                self.buffer = 0.0;
                self.coyote = 0.0;
                self.grounded = false;
                self.jumping = true;
                self.air_jumps = fp.max_air_jumps;
            } else if engaged {
                self.vel.y = -fp.wall_jump_vy;
                self.vel.x = -(self.wall_dir as f64) * fp.wall_jump_vx;
                self.wall_lock = fp.wall_jump_lock;
                self.facing = -self.wall_dir;
                self.buffer = 0.0;
                self.jumping = true;
                self.air_jumps = fp.max_air_jumps;
            } else if self.air_jumps > 0 {
                self.vel.y = -fp.jump_speed;
                self.air_jumps -= 1;
                self.did_double = true;
                self.buffer = 0.0;
                self.jumping = true;
            }
        }
        // jump must be held while rising to keep low gravity
        if !jump_held || self.vel.y >= 0.0 {
            self.jumping = false;
        }

        // --- horizontal accel / friction ---
        let eff_in = if self.wall_lock > 0.0 { 0.0 } else { in_x };
        if eff_in != 0.0 {
            let accel = if self.grounded { fp.run_accel } else { fp.air_accel };
            self.vel.x += accel * eff_in * dt;
            // clamp only in the input direction (external pushes may exceed max_run)
            if eff_in > 0.0 && self.vel.x > fp.max_run {
                self.vel.x = fp.max_run;
            }
            if eff_in < 0.0 && self.vel.x < -fp.max_run {
                self.vel.x = -fp.max_run;
            }
            self.facing = if eff_in > 0.0 { 1 } else { -1 };
        } else if self.wall_lock <= 0.0 {
            let fr = if self.grounded { fp.ground_friction } else { fp.air_friction };
            let dec = fr * dt;
            if self.vel.x > 0.0 {
                self.vel.x = (self.vel.x - dec).max(0.0);
            } else if self.vel.x < 0.0 {
                self.vel.x = (self.vel.x + dec).min(0.0);
            }
        }

        // --- wall slide detection (before gravity so we can clamp the fall) ---
        let pressing_wall = (in_x > 0.0 && self.wall_dir > 0) || (in_x < 0.0 && self.wall_dir < 0);
        let wall_sliding = !self.grounded && self.wall_dir != 0 && self.vel.y > 0.0 && pressing_wall;

        // --- gravity (dual) ---
        let rising = self.vel.y < 0.0;
        let g = if rising && jump_held && self.jumping {
            fp.gravity_rise
        } else {
            fp.gravity_fall
        };
        self.vel.y += g * dt;
        if down_held && !self.grounded {
            self.vel.y += fp.down_fast_fall * dt;
        }
        if self.vel.y > fp.max_fall {
            self.vel.y = fp.max_fall;
        }
        if wall_sliding && self.vel.y > fp.wall_slide_max_fall {
            self.vel.y = fp.wall_slide_max_fall;
        }

        // --- integrate with collision ---
        self.step_axis(map, self.vel.x * dt, 0.0);
        self.step_axis(map, 0.0, self.vel.y * dt);

        // --- post: recompute contacts + state for rendering/queries ---
        self.grounded = self.detect_grounded(map);
        self.wall_dir = self.detect_wall(map);
        self.state = if self.grounded {
            State::Grounded
        } else if wall_sliding {
            State::WallSliding
        } else {
            State::Airborne
        };
    }

    /// Move along one axis in <=1px sub-steps, stopping (and zeroing that axis'
    /// velocity) on the first solid contact. No tunneling since step < tile size.
    fn step_axis(&mut self, map: &TileMap, dx: f64, dy: f64) {
        let dist = dx.abs() + dy.abs();
        if dist == 0.0 {
            return;
        }
        // NB: f64::signum(0.0) == 1.0, so derive direction explicitly (0 stays 0).
        let sx = if dx > 0.0 { 1.0 } else if dx < 0.0 { -1.0 } else { 0.0 };
        let sy = if dy > 0.0 { 1.0 } else if dy < 0.0 { -1.0 } else { 0.0 };
        let mut rem = dist;
        while rem > 0.0 {
            let s = rem.min(1.0);
            let nx = self.pos.x + sx * s;
            let ny = self.pos.y + sy * s;
            // Solid contact stops on any axis; a one-way platform only stops a
            // descending box whose feet cross its top (jump up through it freely).
            let blocked = map.overlaps(nx, ny, self.w, self.h)
                || (dy > 0.0 && map.lands_on_oneway(nx, self.w, self.pos.y + self.h, ny + self.h));
            if blocked {
                if dx != 0.0 {
                    self.vel.x = 0.0;
                }
                if dy != 0.0 {
                    self.vel.y = 0.0;
                }
                return;
            }
            self.pos.x = nx;
            self.pos.y = ny;
            rem -= s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_world() -> TileMap {
        // 20 wide, floor at row 10
        let rows = [
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "....................",
            "####################",
        ];
        TileMap::from_ascii(&rows)
    }

    #[test]
    fn falls_and_lands() {
        let map = flat_world();
        let fp = FeelParams::default();
        let mut p = Player::new(32.0, 16.0);
        for _ in 0..240 {
            p.step(&map, 1.0 / 60.0, 0.0, false, false, false, &fp);
        }
        // floor top is at y = 10*16 = 160; player h=16 so rests near y=144.
        assert!(p.grounded, "should be grounded after falling");
        assert!((p.pos.y - 144.0).abs() < 2.0, "rest y was {}", p.pos.y);
    }

    #[test]
    fn jump_goes_up_then_returns() {
        let map = flat_world();
        let fp = FeelParams::default();
        let mut p = Player::new(32.0, 143.0);
        // settle on ground
        for _ in 0..30 {
            p.step(&map, 1.0 / 60.0, 0.0, false, false, false, &fp);
        }
        assert!(p.grounded);
        let y0 = p.pos.y;
        // press jump (hold)
        p.step(&map, 1.0 / 60.0, 0.0, true, true, false, &fp);
        let mut min_y = p.pos.y;
        for _ in 0..30 {
            p.step(&map, 1.0 / 60.0, 0.0, false, true, false, &fp);
            min_y = min_y.min(p.pos.y);
        }
        assert!(min_y < y0 - 20.0, "should rise; min_y={} y0={}", min_y, y0);
    }

    #[test]
    fn no_horizontal_drift_while_falling() {
        let map = flat_world();
        let fp = FeelParams::default();
        let mut p = Player::new(64.0, 16.0);
        let x0 = p.pos.x;
        for _ in 0..120 {
            p.step(&map, 1.0 / 60.0, 0.0, false, false, false, &fp);
        }
        assert!((p.pos.x - x0).abs() < 0.001, "no input → no x drift; drifted to {}", p.pos.x);
    }

    #[test]
    fn double_jump_available_in_air() {
        let map = flat_world();
        let fp = FeelParams::default();
        let mut p = Player::new(32.0, 143.0);
        for _ in 0..30 {
            p.step(&map, 1.0 / 60.0, 0.0, false, false, false, &fp);
        }
        // first jump
        p.step(&map, 1.0 / 60.0, 0.0, true, true, false, &fp);
        assert_eq!(p.air_jumps, fp.max_air_jumps);
        // rise a bit
        for _ in 0..10 {
            p.step(&map, 1.0 / 60.0, 0.0, false, true, false, &fp);
        }
        let before = p.air_jumps;
        // second (double) jump
        p.step(&map, 1.0 / 60.0, 0.0, true, true, false, &fp);
        assert_eq!(p.air_jumps, before - 1, "double jump should consume an air jump");
        assert!(p.did_double);
    }
}

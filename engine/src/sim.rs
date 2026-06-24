//! The tick-driven simulation core (RECORD_REPLAY.md).
//!
//! [`Sim`] owns the player + effects and advances them exactly **one `Player::step`
//! per tick**. Crucially, every time value it uses — effect spawn timestamps, the
//! wall-slide spark throttle, effect expiry — is derived from a **tick clock**
//! (`tick * SIM_DT_NS`), never `now_ns()`. That is what makes a run reproducible:
//! feed the same per-tick [`InputFrame`]s against the same arena and you get the
//! same positions, the same effects, and the same `mono_text` keyframes — whether
//! played live at 60 fps or replayed headless as fast as the CPU allows.
//!
//! Wall-clock survives in the live loop only for things that never appear in a
//! snapshot: frame pacing, the FPS readout, and inter-tick render interpolation.

use crate::capture::InputFrame;
use crate::effects::{self, Effects};
use crate::math::Vec2;
use crate::player::{FeelParams, Player, State};
use crate::time::NS_PER_SEC;
use crate::world::TileMap;

/// Fixed simulation timestep: 60 Hz.
pub const SIM_DT: f64 = 1.0 / 60.0;
pub const SIM_DT_NS: u64 = NS_PER_SEC / 60;

/// Wall-slide sparks re-emit at ~18 Hz while sliding (was wall-clock; now ticks).
const SPARK_PERIOD_NS: u64 = NS_PER_SEC / 18;

pub struct Sim {
    pub player: Player,
    pub fx: Effects,
    pub tick: u64,
    pub prev_pos: Vec2, // player position before the most recent tick (for render lerp)
    pub fp: FeelParams,
    pub last_input: InputFrame, // most recent input (pose selection reads down_held)
    spawn: (f64, f64),
    was_double: bool,
    was_grounded: bool,
    last_spark_clock: u64,
}

impl Sim {
    pub fn new(player: Player, spawn: (f64, f64)) -> Self {
        let prev_pos = player.pos;
        Sim {
            player,
            fx: Effects::new(),
            tick: 0,
            prev_pos,
            fp: FeelParams::default(),
            last_input: InputFrame::default(),
            spawn,
            was_double: false,
            was_grounded: true,
            last_spark_clock: 0,
        }
    }

    /// The tick clock in ns — the single time source for animation + effects.
    #[inline]
    pub fn clock(&self) -> u64 {
        self.tick * SIM_DT_NS
    }

    /// Advance one fixed tick: physics, then the same event-triggered effects the
    /// live loop used to spawn inline (double-jump puff, landing dust, wall-slide
    /// sparks), all timestamped on the tick clock.
    pub fn step(&mut self, map: &TileMap, inp: InputFrame) {
        self.prev_pos = self.player.pos;
        self.last_input = inp;
        let clock = self.clock();

        self.player.step(
            map,
            SIM_DT,
            inp.axis_x as f64,
            inp.jump_pressed,
            inp.jump_held,
            inp.down_held,
            &self.fp,
        );

        // Event-triggered effects — detected on the rising edge so a jump-and-land
        // inside one render frame still fires (did_double resets on landing).
        let p = &self.player;
        if p.did_double && !self.was_double {
            self.fx.spawn(&effects::PUFF, p.pos.x + p.w / 2.0, p.pos.y + p.h, clock);
        }
        if p.grounded && !self.was_grounded {
            self.fx.spawn(&effects::DUST, p.pos.x + p.w / 2.0, p.pos.y + p.h, clock);
        }
        self.was_double = p.did_double;
        self.was_grounded = p.grounded;

        // Continuous friction sparks straddling the wall-contact line, throttled on
        // the tick clock so they're identical live and on replay.
        if p.state == State::WallSliding && clock.saturating_sub(self.last_spark_clock) >= SPARK_PERIOD_NS {
            let sx = p.pos.x + p.w / 2.0 + p.wall_dir as f64 * p.w / 2.0;
            let sy = p.pos.y + p.h * 0.6;
            self.fx.spawn(&effects::SPARK, sx, sy, clock);
            self.last_spark_clock = clock;
        }

        // Safety net (shouldn't happen in a closed box): respawn if it escapes,
        // preserving the Munchii-fitted hitbox size.
        if self.player.pos.y > map.px_h() + 64.0 {
            let (w, h) = (self.player.w, self.player.h);
            self.player = Player::new(self.spawn.0, self.spawn.1);
            self.player.w = w;
            self.player.h = h;
            self.prev_pos = self.player.pos;
        }

        self.tick += 1;
        self.fx.update(self.clock());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::TILE;

    fn flat() -> TileMap {
        TileMap::from_ascii(&[
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
        ])
    }

    fn scripted_inputs() -> Vec<InputFrame> {
        // A deterministic, mixed script: run right, jump, hold, double-jump, fall.
        let mut v = Vec::new();
        for i in 0..120u64 {
            v.push(InputFrame {
                axis_x: if i % 7 < 4 { 1 } else { -1 },
                jump_pressed: i == 10 || i == 25,
                jump_held: (10..20).contains(&i) || (25..35).contains(&i),
                down_held: i % 11 == 0,
            });
        }
        v
    }

    #[test]
    fn sim_matches_raw_player_step() {
        // Sim must not perturb the physics: stepping the Sim and stepping a bare
        // Player with the same inputs must land in the exact same place.
        let map = flat();
        let inputs = scripted_inputs();

        let mut sim = Sim::new(Player::new(2.0 * TILE, 2.0 * TILE), (2.0 * TILE, 2.0 * TILE));
        let mut raw = Player::new(2.0 * TILE, 2.0 * TILE);
        let fp = FeelParams::default();
        for &inp in &inputs {
            sim.step(&map, inp);
            raw.step(&map, SIM_DT, inp.axis_x as f64, inp.jump_pressed, inp.jump_held, inp.down_held, &fp);
        }
        assert_eq!(sim.player.pos, raw.pos, "Sim physics drifted from bare Player::step");
        assert_eq!(sim.tick, inputs.len() as u64);
    }

    #[test]
    fn replay_is_bit_identical() {
        // The determinism guarantee: the same inputs against the same arena yield
        // identical position, velocity, and live-effect count, run twice.
        let map = flat();
        let inputs = scripted_inputs();
        let run = || {
            let mut sim = Sim::new(Player::new(2.0 * TILE, 2.0 * TILE), (2.0 * TILE, 2.0 * TILE));
            for &inp in &inputs {
                sim.step(&map, inp);
            }
            (sim.player.pos, sim.player.vel, sim.fx.render(sim.clock()).len())
        };
        assert_eq!(run(), run());
    }
}

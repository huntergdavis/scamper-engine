//! Effects layer: small ASCII clips that game events spawn on top of the
//! sprite animations and that expire on their own.
//!
//! An [`Effect`] is a reusable clip (frames + speed + tint). The [`Effects`]
//! manager holds the *live* instances — each pinned to a world position and a
//! start time — advances them by wall-clock, and drops them when their clip
//! finishes. Effects are world-anchored (a puff stays where it was spawned, the
//! character moves on). The character backends composite them via `Overlay`.

use crate::time::NS_PER_SEC;

/// A reusable effect clip. `frames` play once, front to back, at `fps`; the
/// clip's lifetime is `frames.len() / fps` seconds. `tint` is the color the
/// colored backends draw it in (the mono tier ignores it). `z` is the draw
/// depth vs the player (whose layer is `z = 0`): negative draws behind him,
/// positive in front (over walls, sprites, even menus).
pub struct Effect {
    pub name: &'static str,
    pub fps: u32,
    pub tint: (u8, u8, u8),
    pub z: i32,
    pub frames: &'static [&'static [&'static str]],
}

// Double-jump propellant burst — pops under the feet and dissipates upward.
pub static PUFF: Effect = Effect {
    name: "puff",
    fps: 16,
    tint: (216, 212, 202),
    z: -1, // just behind Munchii
    frames: &[
        &["(*°O°*)", " *   * "],
        &["°o   o°", "  ' '  "],
        &[" ' . ' ", "   .   "],
        &["  . .  ", "       "],
    ],
};

// Wall-slide friction — hot white sparks that scatter up off his feet. Short and
// re-emitted continuously while he slides, so it reads as a steady stream. White
// (not yellow) so it reads as heat/friction against the wall, not a stain.
pub static SPARK: Effect = Effect {
    name: "spark",
    fps: 20,
    tint: (250, 250, 255),
    z: 1, // pops in front
    frames: &[
        &[" * ", "* *"],
        &["* *", " ° "],
        &["°.°", "   "],
        &[" . ", "   "],
    ],
};

// Landing dust — a low scuff that kicks out and settles.
pub static DUST: Effect = Effect {
    name: "dust",
    fps: 12,
    tint: (198, 182, 150),
    z: -1,
    frames: &[
        &[" _   _ ", "(_) (_)"],
        &["°  .  °", " '   ' "],
        &["  ' '  ", "       "],
    ],
};

// Block bonk — a quick scuff that kicks up off the block's top.
pub static BONK: Effect = Effect {
    name: "bonk",
    fps: 18,
    tint: (226, 214, 188),
    z: 1,
    frames: &[&[" .. ", "    "], &["°  °", " .. "], &["    ", "    "]],
};

// A startled exclamation — bonking something solid that won't budge.
pub static BANG: Effect = Effect {
    name: "bang",
    fps: 9,
    tint: (255, 232, 120),
    z: 2,
    frames: &[&[" ! "], &[" ! "], &[" ! "], &[" . "]],
};

// Enemy bop — a bright burst when a critter pops into a treat.
pub static BOP: Effect = Effect {
    name: "bop",
    fps: 18,
    tint: (255, 236, 150),
    z: 2,
    frames: &[&[" \\*/ ", " /.\\ "], &["* . *", " . . "], &[" . . ", "     "]],
};

// Collect / block-release sparkle.
pub static SPARKLE: Effect = Effect {
    name: "sparkle",
    fps: 16,
    tint: (255, 245, 180),
    z: 2,
    frames: &[&[" + "], &["(+)"], &[" * "], &[" . "]],
};

// Level complete — a little cheer of rising stars.
pub static CHEER: Effect = Effect {
    name: "cheer",
    fps: 8,
    tint: (255, 240, 160),
    z: 3,
    frames: &[&["* . *", " . . "], &[" * * ", "*   *"], &["  *  ", " * * "], &[" . . ", "     "]],
};

struct Active {
    fx: &'static Effect,
    x: f64, // world (framebuffer-px) anchor: horizontal center
    y: f64, // world anchor: top of the clip
    start: u64,
}

/// The set of currently-playing effect instances.
#[derive(Default)]
pub struct Effects {
    active: Vec<Active>,
}

impl Effects {
    pub fn new() -> Self {
        Effects { active: Vec::new() }
    }

    /// Spawn `fx` anchored at world point (`x`, `y`) (horizontal center, top).
    pub fn spawn(&mut self, fx: &'static Effect, x: f64, y: f64, now: u64) {
        self.active.push(Active { fx, x, y, start: now });
    }

    /// Drop effects whose clip has finished.
    pub fn update(&mut self, now: u64) {
        self.active.retain(|a| {
            let step = NS_PER_SEC / a.fx.fps.max(1) as u64;
            now.saturating_sub(a.start) < a.fx.frames.len() as u64 * step
        });
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// For each live effect: (current frame, tint, z, center-x, top-y in world px).
    pub fn render(&self, now: u64) -> Vec<(&'static [&'static str], (u8, u8, u8), i32, f64, f64)> {
        self.active
            .iter()
            .filter(|a| !a.fx.frames.is_empty()) // a clip with no frames can't index
            .map(|a| {
                let step = NS_PER_SEC / a.fx.fps.max(1) as u64;
                let i = ((now.saturating_sub(a.start) / step) as usize).min(a.fx.frames.len() - 1);
                (a.fx.frames[i], a.fx.tint, a.fx.z, a.x, a.y)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_expire_after_their_clip() {
        let mut fx = Effects::new();
        fx.spawn(&PUFF, 0.0, 0.0, 0);
        assert!(!fx.is_empty());
        let life = PUFF.frames.len() as u64 * (NS_PER_SEC / PUFF.fps as u64);
        fx.update(life / 2);
        assert!(!fx.is_empty(), "still playing mid-clip");
        fx.update(life + 1);
        assert!(fx.is_empty(), "should be gone after the clip ends");
    }

    #[test]
    fn render_advances_then_holds_last_frame() {
        let mut fx = Effects::new();
        fx.spawn(&PUFF, 10.0, 20.0, 0);
        let r = fx.render(0)[0];
        assert_eq!(r.0, PUFF.frames[0]);
        assert_eq!((r.1, r.2, r.3, r.4), (PUFF.tint, PUFF.z, 10.0, 20.0));
    }
}

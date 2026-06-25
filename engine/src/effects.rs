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

// Spiky pop — burst scatter of little shards (a prickle bursting). The `x`/`+`
// shards read as a sharp shatter, distinct from the soft BOP, even in mono.
pub static SHARDS: Effect = Effect {
    name: "shards",
    fps: 18,
    tint: (228, 150, 210),
    z: 2,
    frames: &[&[" x ", "x x"], &["x x", " + "], &["+ +", "   "], &[" . ", "   "]],
};

// Enemy bop — a bright burst when a critter pops into a treat.
pub static BOP: Effect = Effect {
    name: "bop",
    fps: 18,
    tint: (255, 236, 150),
    z: 2,
    frames: &[&[" \\*/ ", " /.\\ "], &["* . *", " . . "], &[" . . ", "     "]],
};

// A kibble-coin spat up out of a block: it leaps, spins (round → edge-on → round),
// and winks out at the top of its arc. The "$" reads as a coin even in mono.
pub static COIN: Effect = Effect {
    name: "coin",
    fps: 14,
    tint: (255, 214, 96),
    z: 2,
    frames: &[
        &["   ", "($)", "   "],
        &["($)", "   ", "   "],
        &["(|)", "   ", "   "],
        &["( )", " ' ", "   "],
        &[" ' ", "   ", "   "],
    ],
};

// Glide feather — a soft tuft that drifts down while gliding on the Flutter
// Collar. The `~`/`,` glyphs read as a wisp even in mono.
pub static FEATHER: Effect = Effect {
    name: "feather",
    fps: 8,
    tint: (200, 232, 255),
    z: -1,
    frames: &[&["~"], &[","], &["."], &[" "]],
};

// Speed-burst dash trail — motion streaks that flick off behind a sprinting hero
// (the Zoomies Treat buff). The chevrons read as speed lines even in mono.
pub static DASH: Effect = Effect {
    name: "dash",
    fps: 20,
    tint: (255, 226, 130),
    z: -1, // behind the hero
    frames: &[&["»»"], &["» "], &["· "]],
};

// Bubble-gear aura — a soap bubble that wobbles up off Munchii and pops. Emitted
// continuously while he wears the Bubble Bone, so that tier reads distinctly (the
// round "o(O)" glyphs show even in mono B&W) from plain Big gear.
pub static BUBBLE: Effect = Effect {
    name: "bubble",
    fps: 10,
    tint: (170, 220, 255),
    z: 2,
    frames: &[&["o", " "], &["O", "°"], &["(O)", " ' "], &[" ° ", "   "]],
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

/// A floating word shout ("BONK!", "WOAH!") — drawn like an effect but carrying a
/// translatable text line (see [`crate::strings`]) rather than ASCII-art frames.
/// It drifts upward as it ages, then vanishes.
struct WordPop {
    text: &'static str,
    tint: (u8, u8, u8),
    x: f64, // world anchor: horizontal center
    y: f64, // world anchor: baseline at spawn
    start: u64,
}

/// Word pops play at this cadence for this many frames (≈ lifetime in seconds =
/// frames / fps), rising `WORD_RISE` px per frame.
const WORD_FPS: u32 = 12;
const WORD_FRAMES: u64 = 9;
const WORD_RISE: f64 = 1.6;

/// The set of currently-playing effect instances.
#[derive(Default)]
pub struct Effects {
    active: Vec<Active>,
    words: Vec<WordPop>,
}

impl Effects {
    pub fn new() -> Self {
        Effects { active: Vec::new(), words: Vec::new() }
    }

    /// Spawn `fx` anchored at world point (`x`, `y`) (horizontal center, top).
    pub fn spawn(&mut self, fx: &'static Effect, x: f64, y: f64, now: u64) {
        self.active.push(Active { fx, x, y, start: now });
    }

    /// Spawn a floating word shout (e.g. `strings::t("fx.bonk")`) in `tint`,
    /// centered at world point (`x`, `y`). It rises and fades on its own.
    pub fn spawn_word(&mut self, text: &'static str, tint: (u8, u8, u8), x: f64, y: f64, now: u64) {
        self.words.push(WordPop { text, tint, x, y, start: now });
    }

    /// Drop effects and word pops whose lifetime has finished.
    pub fn update(&mut self, now: u64) {
        self.active.retain(|a| {
            let step = NS_PER_SEC / a.fx.fps.max(1) as u64;
            now.saturating_sub(a.start) < a.fx.frames.len() as u64 * step
        });
        let wstep = NS_PER_SEC / WORD_FPS as u64;
        self.words.retain(|w| now.saturating_sub(w.start) < WORD_FRAMES * wstep);
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty() && self.words.is_empty()
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

    /// Live word pops: (text, tint, z, center-x, top-y in world px). They rise as
    /// they age and draw in front of everything (`z` is high).
    pub fn render_words(&self, now: u64) -> Vec<(&'static str, (u8, u8, u8), i32, f64, f64)> {
        let step = NS_PER_SEC / WORD_FPS as u64;
        self.words
            .iter()
            .filter_map(|w| {
                let frame = now.saturating_sub(w.start) / step;
                if frame >= WORD_FRAMES {
                    return None;
                }
                Some((w.text, w.tint, 1500, w.x, w.y - frame as f64 * WORD_RISE))
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
    fn word_pops_rise_then_expire() {
        let mut fx = Effects::new();
        fx.spawn_word("BONK!", (255, 255, 255), 100.0, 50.0, 0);
        let r0 = fx.render_words(0);
        assert_eq!(r0.len(), 1);
        assert_eq!(r0[0].0, "BONK!");
        assert_eq!((r0[0].3, r0[0].4), (100.0, 50.0), "starts at its anchor");
        // A few frames later it has drifted upward (smaller y).
        let step = NS_PER_SEC / WORD_FPS as u64;
        let later = fx.render_words(step * 3);
        assert!(later[0].4 < 50.0, "word rises as it ages");
        // After its lifetime it's gone.
        fx.update(step * (WORD_FRAMES + 1));
        assert!(fx.is_empty());
        assert!(fx.render_words(step * (WORD_FRAMES + 1)).is_empty());
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

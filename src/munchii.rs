//! Munchii — the ascii-tier player sprite and his animation frames.
//!
//! Each frame is 6 text rows on a fixed grid; every animation is a full
//! 6-frame cycle so motion reads smoothly. The tail's first column carries
//! mood (wags fast when happy, holds still when hurt); the parachute ears carry
//! health. The `ascii`/`mono` backends draw these directly; the higher tiers
//! (8-bit, cartoon) are separate, sharper assets.

pub struct Anim {
    pub name: &'static str,
    pub fps: u32,
    pub frames: &'static [&'static [&'static str]],
}

/// Nominal sprite cell size (the standing frames); used to scale the character
/// consistently across backends.
pub const W: usize = 19;
pub const H: usize = 6;

/// Look an animation up by name (falls back to idle).
pub fn anim(name: &str) -> &'static Anim {
    ALL.iter().find(|a| a.name == name).unwrap_or(&ALL[0])
}

/// Beagle palette for a sprite glyph: brown fur, white muzzle/belly, black
/// nose + eyes. Used by the colored-ASCII backend.
pub fn beagle_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '@' | 'o' | '^' | 'x' | 'X' | '-' => (28, 22, 18), // nose / eye — near-black
        '=' | '_' => (238, 230, 214),                       // muzzle / belly — white
        _ => (156, 102, 58),                                // fur / ears / legs / tail — brown
    }
}

// tail sways, one slow blink
const IDLE: &[&[&str]] = &[
    &["             __    ", " )         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " )         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " |         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " \\         (( -==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " \\         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " |         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
];

// moving right: legs shuffle, tail wags
const WALK: &[&[&str]] = &[
    &["             __    ", " )         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " |         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n       n  n  "],
    &["             __    ", " /         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "   n  n      n  n  "],
    &["             __    ", " |         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "   n  n     n  n   "],
    &["             __    ", " \\         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n     n  n    "],
    &["             __    ", " |         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
];

// a jump arc: gather, launch, rise, ear-flutter, flutter, fall
const JUMP: &[&[&str]] = &[
    &["            __     ", " |        (( o==@  ", " |        (\\_)     ", " /__________\\      ", " \\__________/      ", "  w        w       "],
    &["          \\(  )/   ", " |        ( o==@   ", "          (\\_)     ", "  \\______/         ", "   \\____/          ", "   u      u        "],
    &["          \\(   )/  ", "           ( o==@  ", "           (\\_)    ", "   \\______/        ", "    \\____/         ", "    u    u         "],
    &["          /(   )\\  ", "           ( ^==@  ", "           (\\_)    ", "   \\______/        ", "    \\____/         ", "    u    u         "],
    &["          \\(   )/  ", "           ( o==@  ", "           (\\_)    ", "   \\______/        ", "    \\____/         ", "    u    u         "],
    &["           \\| |/   ", "           ( o==@  ", "           (\\_)    ", "   \\______/        ", "    \\____/         ", "   n      n        "],
];

// double jump: frame 0 is the propellant burst right under his feet; it
// dissipates over the next few frames as he rides the arc up. Played once,
// triggered the instant the second jump fires (not looped).
const DBLJUMP: &[&[&str]] = &[
    &["    \\(   )/  ", "    ( O==@   ", "    (\\_)     ", "   \\_____/   ", "   *u   u*   ", "  (*°O°*)    "],
    &["    /(   )\\  ", "    ( o==@   ", "    (\\_)     ", "   \\_____/   ", "    u   u    ", "   °o   o°   "],
    &["    \\(   )/  ", "    ( o==@   ", "    (\\_)     ", "   \\_____/   ", "    u   u    ", "    '   '    "],
    &["    /(   )\\  ", "    ( o==@   ", "    (\\_)     ", "   \\_____/   ", "    u   u    ", "     . .     "],
    &["    \\(   )/  ", "    ( o==@   ", "    (\\_)     ", "   \\_____/   ", "    u   u    ", "             "],
    &["     \\| |/   ", "    ( o==@   ", "    (\\_)     ", "   \\_____/   ", "    u   u    ", "             "],
];

// low + compact: head out front, short body, back feet + front feet + tail
const CRAWL: &[&[&str]] = &[
    &["                   ", "                   ", "            __     ", " )    ___(( o==@   ", " |       (\\_)      ", "   w  w    n  n    "],
    &["                   ", "                   ", "            __     ", " |    ___(( o==@   ", " |       (\\_)      ", "    w  w    n  n   "],
    &["                   ", "                   ", "            __     ", " /    ___(( o==@   ", " |       (\\_)      ", "   w   w   n   n   "],
    &["                   ", "                   ", "            __     ", " |    ___(( o==@   ", " |       (\\_)      ", "  w  w    n  n     "],
    &["                   ", "                   ", "            __     ", " \\    ___(( o==@   ", " |       (\\_)      ", "   w  w    n  n    "],
    &["                   ", "                   ", "            __     ", " )    ___(( o==@   ", " |       (\\_)      ", "    w  w   n   n   "],
];

// upright, pressed to the wall (renderer flips him to face it); scuff/spark
// puffs fly off his feet on the back (wall) side and rise as he slides — the
// sprite faces right, so sparks sit on the left and flip with him. No rotation.
const WALLSLIDE: &[&[&str]] = &[
    &["             __    ", "           (( o==@ ", "           (\\_)    ", "  /__________\\     ", "  \\__________/     ", "   n  n     n  n   "],
    &["             __    ", "           (( o==@ ", "           (\\_)    ", "  /__________\\     ", "  \\__________/     ", " * n  n     n  n   "],
    &["             __    ", "           (( o==@ ", "           (\\_)    ", "  /__________\\     ", "* \\__________/     ", " . n  n     n  n   "],
    &["             __    ", "           (( o==@ ", "           (\\_)    ", "° /__________\\     ", " '\\__________/     ", "   n  n     n  n   "],
    &["             __    ", "           (( o==@ ", "           (\\_)    ", " '/__________\\     ", ", \\__________/     ", "   n  n     n  n   "],
    &["             __    ", "           (( o==@ ", "           (\\_)    ", "  /__________\\     ", "  \\__________/     ", ".  n  n     n  n   "],
];

// big fast tail wag, steady eye (no blink)
const HAPPY: &[&[&str]] = &[
    &["             __    ", " /         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " |         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " \\         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " |         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " /         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["             __    ", " |         (( o==@ ", " |         (\\_)    ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
];

// drooped short ears, sad eye, tail dead-still — a faint tremble
const HURT: &[&[&str]] = &[
    &["            __     ", " ,         ( x==@  ", "           (.      ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["            __     ", " ,         ( x==@  ", "           (.      ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["            __     ", " ,         ( X==@  ", "           (.      ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["            __     ", " ,         ( x==@  ", "           (.      ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["            __     ", " ,         ( x==@  ", "           (,      ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
    &["            __     ", " ,         ( x==@  ", "           (.      ", " /____________\\    ", " \\____________/    ", "  n  n      n  n   "],
];

/// All animations, in preview order. fps doubles as personality: happy wags
/// fast, hurt barely moves.
pub const ALL: &[Anim] = &[
    Anim { name: "idle", fps: 4, frames: IDLE },
    Anim { name: "walk", fps: 9, frames: WALK },
    Anim { name: "jump", fps: 9, frames: JUMP },
    Anim { name: "double-jump", fps: 8, frames: DBLJUMP },
    Anim { name: "crawl", fps: 7, frames: CRAWL },
    Anim { name: "wall-slide", fps: 6, frames: WALLSLIDE },
    Anim { name: "happy", fps: 14, frames: HAPPY },
    Anim { name: "hurt", fps: 3, frames: HURT },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_anim_has_nonempty_6_row_frames() {
        // Frame count is free (animations run their natural length); only the
        // row height is fixed so frames composite consistently.
        for a in ALL {
            assert!(!a.frames.is_empty(), "{} has no frames", a.name);
            for (i, f) in a.frames.iter().enumerate() {
                assert_eq!(f.len(), 6, "{} frame {i} is not 6 rows", a.name);
            }
        }
    }

    #[test]
    fn frames_within_an_anim_share_width() {
        for a in ALL {
            let w = a.frames[0].iter().map(|l| l.chars().count()).max().unwrap();
            for f in a.frames {
                for line in *f {
                    assert!(line.chars().count() <= w, "{} row too wide", a.name);
                }
            }
        }
    }
}

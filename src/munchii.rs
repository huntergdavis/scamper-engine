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

// double jump: the second jump fires a subtle fart puff below that propels him
const DBLJUMP: &[&[&str]] = &[
    &["    \\(   )/  ", "    ( o==@   ", "    (\\_)     ", "   \\_____/   ", "    u   u    ", "   (° °)     "],
    &["    /(   )\\  ", "    ( o==@   ", "    (\\_)     ", "   \\_____/   ", "    u   u    ", "   ~ o ~     "],
    &["    \\(   )/  ", "    ( o==@   ", "    (\\_)     ", "   \\_____/   ", "    u   u    ", "    ' '      "],
    &["    /(   )\\  ", "    ( o==@   ", "    (\\_)     ", "   \\_____/   ", "    u   u    ", "     .       "],
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
    fn every_anim_is_a_6_frame_cycle_of_6_row_frames() {
        for a in ALL {
            assert_eq!(a.frames.len(), 6, "{} is not a 6-frame cycle", a.name);
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

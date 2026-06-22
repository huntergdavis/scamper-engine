//! Munchii — the ascii-tier player sprite and his animation frames.
//!
//! Each frame is 6 text rows on a fixed grid (built bottom-up: feet, two body
//! rows, head+tail, ears+tail-tip), so frames swap without jumping. Posing a
//! behaviour is just nudging rows — the tail's first column carries mood, the
//! ears carry health. The `ascii`/`mono` backends draw these directly; the
//! higher tiers (8-bit, cartoon) are separate, sharper assets.

pub struct Anim {
    pub name: &'static str,
    pub fps: u32,
    pub frames: &'static [&'static [&'static str]],
}

// moving right: legs shuffle, tail wags
const WALK: &[&[&str]] = &[
    &[
        "             __   ",
        " )          ( o==@",
        " |          (_,   ",
        " /____________\\   ",
        " \\____________/   ",
        "  n  n      n  n  ",
    ],
    &[
        "             __   ",
        " |          ( o==@",
        " |          (_,   ",
        " /____________\\   ",
        " \\____________/   ",
        "   n  n    n  n   ",
    ],
    &[
        "             __   ",
        " /          ( o==@",
        " |          (_,   ",
        " /____________\\   ",
        " \\____________/   ",
        "  n   n    n   n  ",
    ],
    &[
        "             __   ",
        " |          ( o==@",
        " |          (_,   ",
        " /____________\\   ",
        " \\____________/   ",
        " n  n      n  n   ",
    ],
];

// tail sways, slow blink
const IDLE: &[&[&str]] = &[
    &[
        "             __   ",
        " )          ( o==@",
        " |          (_,   ",
        " /____________\\   ",
        " \\____________/   ",
        "  n  n      n  n  ",
    ],
    &[
        "             __   ",
        " \\          ( o==@",
        "  \\         (_,   ",
        " /____________\\   ",
        " \\____________/   ",
        "  n  n      n  n  ",
    ],
    &[
        "             __   ",
        " )          ( -==@",
        " |          (_,   ",
        " /____________\\   ",
        " \\____________/   ",
        "  n  n      n  n  ",
    ],
];

// ears up, tail straight up, eye wide, feet tucked
const JUMP: &[&[&str]] = &[&[
    " |          \\\\ // ",
    " |          ( O==@",
    "            (_,   ",
    "  \\________/      ",
    "   \\______/       ",
    "   u      u       ",
]];

// head sinks into a flat wide body, phat + low, tiny feet shuffle
const CRAWL: &[&[&str]] = &[
    &[
        "                 ",
        "        ____     ",
        " )    _( o==@_   ",
        " |   /  (_,    \\ ",
        "     \\_________/ ",
        "      w  w   w w ",
    ],
    &[
        "                 ",
        "        ____     ",
        " )    _( o==@_   ",
        " |   /  (_,    \\ ",
        "     \\_________/ ",
        "     w   w  w  w ",
    ],
];

// pressed to a wall (right), ears + tail streaming up in the slide
const WALLSLIDE: &[&[&str]] = &[
    &[
        "        __    |",
        "     ( o==@   |",
        "     (_,`     |",
        "   /________  |",
        "   \\________  |",
        "    n    n    |",
    ],
    &[
        "        __   ~|",
        "     ( o==@   |",
        "     (_,`    ~|",
        "   /________  |",
        "   \\________ ~|",
        "    n   n     |",
    ],
];

// emotions (ears = health)
const HAPPY: &[&[&str]] = &[&[
    "            __   ",
    " /         ((o==@",
    " |         ((_,  ",
    " /____________\\  ",
    " \\____________/  ",
    "  n  n      n  n ",
]];

const HURT: &[&[&str]] = &[&[
    "            __   ",
    " ,         ( x==@",
    "           (.    ",
    " /____________\\  ",
    " \\____________/  ",
    "  n  n      n  n ",
]];

/// All animations, in preview order.
pub const ALL: &[Anim] = &[
    Anim { name: "idle", fps: 2, frames: IDLE },
    Anim { name: "walk", fps: 6, frames: WALK },
    Anim { name: "jump", fps: 3, frames: JUMP },
    Anim { name: "crawl", fps: 4, frames: CRAWL },
    Anim { name: "wall-slide", fps: 5, frames: WALLSLIDE },
    Anim { name: "happy", fps: 1, frames: HAPPY },
    Anim { name: "hurt", fps: 1, frames: HURT },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_anim_has_consistent_6_row_frames() {
        for a in ALL {
            assert!(!a.frames.is_empty(), "{} has no frames", a.name);
            for (i, f) in a.frames.iter().enumerate() {
                assert_eq!(f.len(), 6, "{} frame {i} is not 6 rows", a.name);
            }
        }
    }

    #[test]
    fn frames_within_an_anim_share_width() {
        // equal-width rows keep the animation from jittering horizontally.
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

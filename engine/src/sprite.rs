//! Sprite registry — the engine's shared character-art library (CAMPAIGN_PLAN §3,
//! §7-§8). Generalizes [`crate::munchii`] into a table of [`Sprite`]s keyed by id,
//! each carrying its glyph-frame animations and a per-glyph palette. Games look a
//! sprite up by id and render it through any backend (the same glyph-frames feed
//! the character tiers directly and rasterize to blocks for the pixel tiers).
//!
//! Art here is **original** and intentionally simple ascii: distinct silhouettes
//! first, polish later. The Mario archetypes some of these reskin are only design
//! templates — the names, look, and behavior are ours (a non-violent dog world).

use crate::munchii::{self, Anim};

/// What a sprite is, for the runtime's sake (how it's spawned / collided).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The player avatar.
    Player,
    /// A critter Munchii pounces (it pops into a treat).
    Creature,
    /// A collectible / power-up.
    Item,
}

/// One character in the bestiary/asset library: animations + how to color them.
pub struct Sprite {
    pub id: &'static str,
    pub role: Role,
    /// Nominal cell size (width in columns, height in rows) used to scale the
    /// sprite consistently across backends; every frame is exactly `h` rows.
    pub w: usize,
    pub h: usize,
    pub anims: &'static [Anim],
    /// Glyph → RGB for the colored backends (each sprite owns its palette).
    pub palette: fn(char) -> (u8, u8, u8),
}

impl Sprite {
    /// Look up an animation by name, falling back to the first one.
    pub fn anim(&self, name: &str) -> &'static Anim {
        self.anims.iter().find(|a| a.name == name).unwrap_or(&self.anims[0])
    }
}

/// The whole registry, in preview order (Munchii first, then critters, items).
pub const ALL: &[Sprite] = &[
    Sprite { id: "munchii", role: Role::Player, w: munchii::W, h: munchii::H, anims: munchii::ALL, palette: munchii::beagle_rgb },
    Sprite { id: "boneling", role: Role::Creature, w: 7, h: 3, anims: BONELING, palette: bone_rgb },
    Sprite { id: "rollo", role: Role::Creature, w: 6, h: 2, anims: ROLLO, palette: rollo_rgb },
    Sprite { id: "kibble", role: Role::Item, w: 4, h: 2, anims: KIBBLE, palette: kibble_rgb },
    Sprite { id: "big_kibble", role: Role::Item, w: 6, h: 3, anims: BIG_KIBBLE, palette: kibble_rgb },
    Sprite { id: "sudsball", role: Role::Item, w: 3, h: 1, anims: SUDSBALL, palette: suds_rgb },
    Sprite { id: "baron_whiskers", role: Role::Creature, w: 8, h: 4, anims: BARON, palette: baron_rgb },
    Sprite { id: "bath_plug", role: Role::Item, w: 3, h: 2, anims: BATH_PLUG, palette: plug_rgb },
    // The wider bestiary — so imported levels actually populate.
    Sprite { id: "flutterbug", role: Role::Creature, w: 6, h: 2, anims: FLUTTERBUG, palette: bug_rgb },
    Sprite { id: "hoppa", role: Role::Creature, w: 5, h: 2, anims: HOPPA, palette: bug_rgb },
    Sprite { id: "pincher", role: Role::Creature, w: 6, h: 2, anims: PINCHER, palette: crab_rgb },
    Sprite { id: "prickle", role: Role::Creature, w: 5, h: 2, anims: PRICKLE, palette: prickle_rgb },
    Sprite { id: "hardhat", role: Role::Creature, w: 6, h: 2, anims: HARDHAT, palette: hard_rgb },
    Sprite { id: "stick_squirrel", role: Role::Creature, w: 6, h: 3, anims: SQUIRREL, palette: squirrel_rgb },
    Sprite { id: "sudsfish", role: Role::Creature, w: 5, h: 2, anims: FISH, palette: fish_rgb },
    Sprite { id: "puffer", role: Role::Creature, w: 6, h: 2, anims: PUFFER, palette: cloud_rgb },
    Sprite { id: "zoomdisc", role: Role::Creature, w: 6, h: 1, anims: ZOOMDISC, palette: disc_rgb },
    Sprite { id: "dandi", role: Role::Creature, w: 5, h: 2, anims: DANDI, palette: plant_rgb },
    Sprite { id: "bubble_bone", role: Role::Item, w: 4, h: 2, anims: BUBBLE_BONE, palette: bubble_rgb },
    Sprite { id: "zoomies_treat", role: Role::Item, w: 4, h: 2, anims: ZOOMIES, palette: treat_rgb },
    Sprite { id: "lucky_squeaky", role: Role::Item, w: 4, h: 2, anims: LUCKY, palette: treat_rgb },
    Sprite { id: "stick", role: Role::Item, w: 3, h: 1, anims: STICK, palette: squirrel_rgb },
];

/// Look up a sprite by id (the IR entity `kind`), if registered.
pub fn get(id: &str) -> Option<&'static Sprite> {
    ALL.iter().find(|s| s.id == id)
}

// ---- palettes ---------------------------------------------------------------

/// Boneling: a cream chew-bone with dark dot-eyes.
fn bone_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (40, 32, 28),    // eyes
        _ => (236, 230, 212),   // bone
    }
}

/// Rollo: a slate-blue roly-poly pillbug, dark eye.
fn rollo_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (26, 26, 38),    // eye
        _ => (122, 132, 156),   // shell
    }
}

/// Kibble (and big kibble): warm brown nugget, light sparkle/sheen.
fn kibble_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '.' | '*' => (244, 224, 150), // sparkle / sheen
        _ => (150, 96, 52),           // kibble
    }
}

/// Sudsball: a pale soap-bubble projectile, bright rim.
fn suds_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'O' => (235, 245, 255), // highlight
        _ => (190, 220, 245),   // bubble
    }
}

/// Baron Whiskers: a grumpy gray tomcat, dark scowling eyes.
fn baron_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '>' | '<' => (24, 22, 28), // scowling eyes
        _ => (140, 138, 150),      // fur
    }
}

/// Bath plug: dark rubber stopper on a gray chain.
fn plug_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'O' => (54, 50, 50),  // rubber stopper
        _ => (158, 158, 168), // chain
    }
}

// ---- wider bestiary palettes ----
fn bug_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (24, 28, 20),
        _ => (120, 180, 72),
    } // green bug, dark eyes
}
fn crab_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (28, 20, 18),
        _ => (210, 96, 56),
    } // orange crab
}
/// Prickle: a thistle-purple spiked burr — bright spikes, dark scowling eye.
fn prickle_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (18, 14, 22),                    // eye
        '^' | '/' | '\\' => (198, 90, 178),     // bright spikes
        _ => (122, 60, 120),                    // body
    }
}
fn hard_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (20, 20, 24),
        '#' => (214, 180, 72),
        _ => (120, 124, 140),
    } // gray shell, yellow helmet, dark eyes
}
fn squirrel_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (28, 20, 14),
        '\'' => (224, 196, 150),
        _ => (150, 96, 52),
    } // brown fur, tan belly
}
fn fish_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (16, 28, 34),
        _ => (96, 196, 210),
    } // cyan fish, dark eye
}
fn cloud_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '\'' => (120, 170, 235),
        _ => (150, 156, 170),
    } // gray cloud, blue drops
}
fn disc_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '-' => (235, 245, 255),
        _ => (120, 210, 225),
    } // bright frisbee
}
fn plant_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '*' => (236, 206, 72),
        _ => (96, 176, 84),
    } // yellow head, green stem
}
fn bubble_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '=' => (235, 245, 255),
        _ => (150, 210, 235),
    } // soapy chew toy
}
fn treat_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '*' => (255, 236, 150),
        _ => (236, 196, 84),
    } // gold treat
}

// ---- frames (original art) --------------------------------------------------

// Boneling — a toddling chew-bone; knob-ends, two eyes, legs that shuffle.
const BONELING_WALK: &[&[&str]] = &[
    &["  ___  ", "<(o o)>", " n   n "],
    &["  ___  ", "<(o o)>", "  n n  "],
    &["  ___  ", "<(o o)>", " n   n "],
    &["  ___  ", "<(o o)>", "  n n  "],
];
const BONELING: &[Anim] = &[Anim { name: "walk", fps: 6, frames: BONELING_WALK }];

// Rollo — a roly-poly pillbug. The moving slash reads as a roll; `curl` is the
// pounced ball you can nudge.
const ROLLO_WALK: &[&[&str]] = &[
    &["(/==o>", "  vv  "],
    &["(=/=o>", "  vv  "],
    &["(==/o>", "  vv  "],
    &["(=/=o>", "  vv  "],
];
const ROLLO_CURL: &[&[&str]] = &[
    &["(####)", " (##) "],
    &["(####)", " (##) "],
];
const ROLLO: &[Anim] = &[
    Anim { name: "walk", fps: 8, frames: ROLLO_WALK },
    Anim { name: "curl", fps: 4, frames: ROLLO_CURL },
];

// Kibble — a collectible nugget that glints.
const KIBBLE_IDLE: &[&[&str]] = &[
    &[" __ ", "(##)"],
    &[" .. ", "(##)"],
];
const KIBBLE: &[Anim] = &[Anim { name: "idle", fps: 3, frames: KIBBLE_IDLE }];

// Big Kibble — the small→big power-up; a chunkier nugget with a sheen.
const BIG_KIBBLE_IDLE: &[&[&str]] = &[
    &[" ____ ", "(####)", "(####)"],
    &[" _.._ ", "(####)", "(####)"],
];
const BIG_KIBBLE: &[Anim] = &[Anim { name: "idle", fps: 3, frames: BIG_KIBBLE_IDLE }];

// Sudsball — a little soap bubble Munchii lobs in his bubble gear.
const SUDSBALL_IDLE: &[&[&str]] = &[&["(o)"], &["(O)"]];
const SUDSBALL: &[Anim] = &[Anim { name: "fly", fps: 10, frames: SUDSBALL_IDLE }];

// Baron Whiskers — a giant grumpy tomcat boss who paces the tub ledge. Paws
// shuffle as he paces (the only way to "beat" him is to pull the bath plug).
const BARON_WALK: &[&[&str]] = &[
    &[" /\\_/\\  ", "( >_< ) ", "(_____) ", " U    U "],
    &[" /\\_/\\  ", "( >_< ) ", "(_____) ", "  U  U  "],
];
const BARON: &[Anim] = &[Anim { name: "walk", fps: 4, frames: BARON_WALK }];

// Bath plug — the "axe": a rubber stopper on a chain. Reach it to win.
const BATH_PLUG_IDLE: &[&[&str]] = &[&[" O ", "/|\\"]];
const BATH_PLUG: &[Anim] = &[Anim { name: "idle", fps: 1, frames: BATH_PLUG_IDLE }];

// ---- wider bestiary frames (original, compact) ----
const FLUTTERBUG_F: &[&[&str]] = &[&["(\\oo/)", "  vv  "], &["(/oo\\)", "  vv  "]];
const FLUTTERBUG: &[Anim] = &[Anim { name: "walk", fps: 8, frames: FLUTTERBUG_F }];

const HOPPA_F: &[&[&str]] = &[&["(o o)", " ^^^ "], &["(o o)", " /^\\ "]];
const HOPPA: &[Anim] = &[Anim { name: "walk", fps: 7, frames: HOPPA_F }];

const PINCHER_F: &[&[&str]] = &[&[">(oo)<", " m  m "], &[">(oo)<", "  mm  "]];
const PINCHER: &[Anim] = &[Anim { name: "walk", fps: 6, frames: PINCHER_F }];

// Prickle — a spiked burr you must NOT pounce (the spines hurt); pop it with a
// Sudsball. The `^`/`/`/`\` crown reads as spikes even in mono B&W.
const PRICKLE_F: &[&[&str]] = &[&["\\^^^/", "(>o<)"], &["/^^^\\", "(>o<)"]];
const PRICKLE: &[Anim] = &[Anim { name: "walk", fps: 6, frames: PRICKLE_F }];

const HARDHAT_F: &[&[&str]] = &[&["/####\\", "(o)(o)"]];
const HARDHAT: &[Anim] = &[Anim { name: "walk", fps: 4, frames: HARDHAT_F }];

const SQUIRREL_F: &[&[&str]] = &[&[" (oo) ", "<|''|>", " /  \\ "], &[" (oo) ", "<|''|>", "  ||  "]];
const SQUIRREL: &[Anim] = &[Anim { name: "walk", fps: 6, frames: SQUIRREL_F }];

const FISH_F: &[&[&str]] = &[&["<oo>=", "  ^^ "], &["<oo>=", "  vv "]];
const FISH: &[Anim] = &[Anim { name: "walk", fps: 8, frames: FISH_F }];

const PUFFER_F: &[&[&str]] = &[&["(~~~~)", " '''' "], &["(~~~~)", "  ''  "]];
const PUFFER: &[Anim] = &[Anim { name: "walk", fps: 5, frames: PUFFER_F }];

const ZOOMDISC_F: &[&[&str]] = &[&["(====)"], &["(=--=)"]];
const ZOOMDISC: &[Anim] = &[Anim { name: "walk", fps: 12, frames: ZOOMDISC_F }];

const DANDI_F: &[&[&str]] = &[&["(***)", " ||| "], &["(***)", " |.| "]];
const DANDI: &[Anim] = &[Anim { name: "walk", fps: 3, frames: DANDI_F }];

const BUBBLE_BONE_F: &[&[&str]] = &[&["o==o", " <> "], &["o==o", " () "]];
const BUBBLE_BONE: &[Anim] = &[Anim { name: "idle", fps: 4, frames: BUBBLE_BONE_F }];

const ZOOMIES_F: &[&[&str]] = &[&["\\**/", "/**\\"], &["/**\\", "\\**/"]];
const ZOOMIES: &[Anim] = &[Anim { name: "idle", fps: 8, frames: ZOOMIES_F }];

const LUCKY_F: &[&[&str]] = &[&["(>o)", " \\/ "], &["(>o)", " vv "]];
const LUCKY: &[Anim] = &[Anim { name: "idle", fps: 4, frames: LUCKY_F }];

// A tumbling thrown stick (Stick Squirrel's projectile).
const STICK_F: &[&[&str]] = &[&["==="], &["\\=/"]];
const STICK: &[Anim] = &[Anim { name: "fly", fps: 10, frames: STICK_F }];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_ids_are_unique_and_expected() {
        let mut ids: Vec<&str> = ALL.iter().map(|s| s.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate sprite id in registry");
        for id in ["munchii", "boneling", "rollo", "kibble", "big_kibble"] {
            assert!(get(id).is_some(), "missing sprite {id}");
        }
        assert_eq!(get("munchii").unwrap().role, Role::Player);
        assert_eq!(get("boneling").unwrap().role, Role::Creature);
        assert_eq!(get("big_kibble").unwrap().role, Role::Item);
        assert!(get("nonesuch").is_none());
    }

    #[test]
    fn every_frame_matches_the_declared_cell_size() {
        for s in ALL {
            assert!(!s.anims.is_empty(), "{} has no anims", s.id);
            for a in s.anims {
                assert!(!a.frames.is_empty(), "{}/{} has no frames", s.id, a.name);
                for (i, f) in a.frames.iter().enumerate() {
                    assert_eq!(f.len(), s.h, "{}/{} frame {i}: {} rows, expected h={}", s.id, a.name, f.len(), s.h);
                    for line in *f {
                        assert!(line.chars().count() <= s.w, "{}/{} frame {i}: row wider than w={}", s.id, a.name, s.w);
                    }
                }
            }
        }
    }

    #[test]
    fn anim_lookup_falls_back_to_first() {
        let rollo = get("rollo").unwrap();
        assert_eq!(rollo.anim("curl").name, "curl");
        assert_eq!(rollo.anim("nope").name, rollo.anims[0].name);
    }

    #[test]
    fn palettes_distinguish_accent_from_body() {
        // Each creature's eye/sparkle color must differ from its body color, or it
        // would render as a featureless blob on the colored tiers.
        assert_ne!(bone_rgb('o'), bone_rgb('_'));
        assert_ne!(rollo_rgb('o'), rollo_rgb('='));
        assert_ne!(kibble_rgb('.'), kibble_rgb('#'));
    }
}

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

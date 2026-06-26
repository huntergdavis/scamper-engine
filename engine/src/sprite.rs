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
    Sprite { id: "springer", role: Role::Creature, w: 5, h: 2, anims: SPRINGER, palette: springer_rgb },
    Sprite { id: "flutter_collar", role: Role::Item, w: 5, h: 2, anims: FLUTTER_COLLAR, palette: flutter_rgb },
    Sprite { id: "swooper", role: Role::Creature, w: 6, h: 2, anims: SWOOPER, palette: swooper_rgb },
    Sprite { id: "trampoline", role: Role::Creature, w: 5, h: 2, anims: TRAMPOLINE, palette: trampoline_rgb },
    Sprite { id: "lift", role: Role::Creature, w: 6, h: 2, anims: LIFT, palette: lift_rgb },
    Sprite { id: "tram", role: Role::Creature, w: 6, h: 2, anims: TRAM, palette: tram_rgb },
    Sprite { id: "star_bone", role: Role::Item, w: 3, h: 2, anims: STAR_BONE, palette: star_rgb },
    Sprite { id: "grow", role: Role::Item, w: 4, h: 2, anims: GROW, palette: grow_rgb },
    Sprite { id: "shrink", role: Role::Item, w: 4, h: 2, anims: SHRINK, palette: shrink_rgb },
    Sprite { id: "super", role: Role::Item, w: 4, h: 2, anims: SUPER, palette: super_rgb },
    Sprite { id: "chaser", role: Role::Creature, w: 4, h: 2, anims: CHASER, palette: chaser_rgb },
    Sprite { id: "haunt", role: Role::Creature, w: 4, h: 3, anims: HAUNT, palette: haunt_rgb },
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
/// Springer: a grass-green bouncing frog, dark eyes.
fn springer_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (16, 22, 14),    // eyes
        _ => (96, 184, 86),     // green body
    }
}
/// Flutter Collar: a sky-blue winged collar (glide power-up).
fn flutter_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (240, 200, 90),                  // bell
        '\\' | '/' | '"' => (210, 235, 255),    // wings
        _ => (150, 200, 240),
    }
}
/// Swooper: a dusky-mauve moth, dark eyes, paler wings.
fn swooper_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (20, 16, 24),                    // eyes
        '^' | 'v' => (208, 196, 224),           // beating wings
        _ => (150, 120, 168),                   // body
    }
}
/// Trampoline: a teal springy pad — bright surface, darker frame/legs.
fn trampoline_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '=' | '~' => (90, 230, 200),            // springy surface
        _ => (60, 120, 110),                    // frame / legs
    }
}
/// Lift: a warm-grey elevator deck on chains.
fn lift_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '#' | ':' => (196, 188, 170),           // deck
        '|' => (130, 130, 138),                 // chains
        _ => (150, 144, 132),                   // frame
    }
}
/// Tram: a steel deck on rail wheels.
fn tram_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '#' => (198, 200, 210),                 // deck
        'o' => (60, 64, 74),                    // wheels
        _ => (120, 126, 140),                   // rail / frame
    }
}
/// Star Bone: a radiant gold power-up.
fn star_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        '*' | '\\' | '/' => (255, 244, 150),    // twinkle
        _ => (240, 232, 200),                   // bone
    }
}
/// Grow arrow: a warm orange-red "get bigger" up-arrow.
fn grow_rgb(_ch: char) -> (u8, u8, u8) {
    (255, 150, 70)
}
/// Shrink arrow: a cool blue "get smaller" down-arrow.
fn shrink_rgb(_ch: char) -> (u8, u8, u8) {
    (110, 190, 255)
}
/// Super arrow: a hot magenta "max size" double-up-arrow.
fn super_rgb(_ch: char) -> (u8, u8, u8) {
    (255, 120, 200)
}
/// Chaser: a rusty-red hunter — dark eye, pale bared teeth.
fn chaser_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (18, 14, 14),                    // eye
        'w' | 'W' => (232, 220, 206),           // teeth / feet
        _ => (192, 72, 56),                     // body
    }
}
/// Haunt: a pale spectral sheet, dark hollow eyes.
fn haunt_rgb(ch: char) -> (u8, u8, u8) {
    match ch {
        'o' => (40, 40, 60),                    // hollow eyes
        _ => (224, 228, 244),                   // ghostly sheet
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

// Springer — a bouncing frog. The walk frame crouches (legs tucked), the second
// frame stretches as it springs, telegraphing the hop.
const SPRINGER_F: &[&[&str]] = &[&["(o o)", " \\_/ "], &["(o o)", " /\"\\ "]];
const SPRINGER: &[Anim] = &[Anim { name: "walk", fps: 5, frames: SPRINGER_F }];

// Flutter Collar — a winged collar power-up that unlocks gliding. Wings flap.
const FLUTTER_COLLAR_F: &[&[&str]] = &[&["\\(o)/", " \"\" "], &["/(o)\\", " \"\" "]];
const FLUTTER_COLLAR: &[Anim] = &[Anim { name: "idle", fps: 4, frames: FLUTTER_COLLAR_F }];

// Swooper — a dusk-moth that weaves through the air. Wings beat up/down.
const SWOOPER_F: &[&[&str]] = &[&["\\(oo)/", " ^^^^ "], &["/(oo)\\", " vvvv "]];
const SWOOPER: &[Anim] = &[Anim { name: "walk", fps: 9, frames: SWOOPER_F }];

// Trampoline — a springy bounce pad. The surface flexes between frames (taut/sprung).
const TRAMPOLINE_F: &[&[&str]] = &[&["[===]", "/   \\"], &["[~~~]", "/   \\"]];
const TRAMPOLINE: &[Anim] = &[Anim { name: "idle", fps: 3, frames: TRAMPOLINE_F }];

// Lift — a riding elevator platform. A solid deck with chain hangers; the deck
// dots shift to read as motion.
const LIFT_F: &[&[&str]] = &[&["|    |", "[####]"], &["|    |", "[::::]"]];
const LIFT: &[Anim] = &[Anim { name: "idle", fps: 3, frames: LIFT_F }];

// Tram — a horizontal riding platform on a rail. Wheels shuffle as it glides.
const TRAM_F: &[&[&str]] = &[&["[####]", "=o==o="], &["[####]", "=o==o="], &["[####]", "==o=o="]];
const TRAM: &[Anim] = &[Anim { name: "idle", fps: 6, frames: TRAM_F }];

// Star Bone — an invincibility power-up: a glowing winged bone that twinkles.
const STAR_BONE_F: &[&[&str]] = &[&["\\*/", "(=)"], &["/*\\", "(=)"]];
const STAR_BONE: &[Anim] = &[Anim { name: "idle", fps: 6, frames: STAR_BONE_F }];

// Grow — the ▲ power-up that makes Munchii bigger. A bold up-arrow that pulses.
const GROW_F: &[&[&str]] = &[&[" /\\ ", "/||\\"], &[" /\\ ", " || "]];
const GROW: &[Anim] = &[Anim { name: "idle", fps: 4, frames: GROW_F }];

// Shrink — the ▼ power-up that makes him smaller. A bold down-arrow that pulses.
const SHRINK_F: &[&[&str]] = &[&["\\||/", " \\/ "], &[" || ", " \\/ "]];
const SHRINK: &[Anim] = &[Anim { name: "idle", fps: 4, frames: SHRINK_F }];

// Super — the ⏫ double-up-arrow that jumps straight to the biggest size.
const SUPER_F: &[&[&str]] = &[&[" /\\ ", " /\\ "], &["/^^\\", "/||\\"]];
const SUPER: &[Anim] = &[Anim { name: "idle", fps: 6, frames: SUPER_F }];

// Chaser — a snarling critter that hunts Munchii. Bared teeth + angry brows read
// as aggression; the feet shuffle as it scrambles.
const CHASER_F: &[&[&str]] = &[&["\\>o<", "/wWw"], &[">o<\\", "wWw\\"]];
const CHASER: &[Anim] = &[Anim { name: "walk", fps: 10, frames: CHASER_F }];

// Haunt — a shy ghost. Domed sheet with a wavy hem and round eyes; the hem ripples
// between frames so it reads as floating.
const HAUNT_F: &[&[&str]] = &[&[" __ ", "(oo)", "wwww"], &[" __ ", "(oo)", "}{}{"]];
// "shy": eyes squeezed shut (-- ) while Munchii is watching — telegraphs the freeze.
const HAUNT_SHY: &[&[&str]] = &[&[" __ ", "(--)", "wwww"]];
const HAUNT: &[Anim] = &[Anim { name: "walk", fps: 4, frames: HAUNT_F }, Anim { name: "shy", fps: 2, frames: HAUNT_SHY }];

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

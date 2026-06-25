//! Levels: the engine-native level format ([`ir`]) and the offline Godot `.tscn`
//! importer ([`import`]) that produces it. See CAMPAIGN_PLAN.md.

pub mod art;
pub mod import;
pub mod ir;
pub mod slice;
pub mod stitch;
pub mod world;

pub use art::{draw_tile, palette, Palette, Theme};
pub use import::{import_scene_file, import_tscn, Imported};
pub use ir::{pack_levels, parse_pack, Entity, Goal, Level, TileKind, TileSpan};
pub use slice::{slice_fingerprint, slice_level};
pub use stitch::stitch;
pub use world::{camera, LevelWorld, Warp};

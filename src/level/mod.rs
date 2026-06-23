//! Levels: the engine-native level format ([`ir`]) and the offline Godot `.tscn`
//! importer ([`import`]) that produces it. See CAMPAIGN_PLAN.md.

pub mod art;
pub mod import;
pub mod ir;
pub mod world;

pub use art::{draw_tile, palette, Palette, Theme};
pub use import::{import_tscn, Imported};
pub use ir::{Entity, Goal, Level, TileKind, TileSpan};
pub use world::{camera, LevelWorld, Warp};

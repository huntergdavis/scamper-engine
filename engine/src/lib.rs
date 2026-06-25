//! Scamper — a terminal 2D platformer engine that renders via the Kitty graphics
//! protocol. Local-first, single-player, keyboard-only. See PROJECT_PLAN.md.

pub mod backend;
pub mod capture;
pub mod dbg;
pub mod effects;
pub mod framebuffer;
pub mod input;
pub mod kitty;
pub mod level;
pub mod math;
pub mod mob;
pub mod munchii;
pub mod player;
pub mod png;
pub mod sprite;
pub mod sim;
pub mod strings;
pub mod terminal;
pub mod time;
pub mod world;

pub use framebuffer::{Framebuffer, Rgba};
pub use math::{vec2, Vec2};

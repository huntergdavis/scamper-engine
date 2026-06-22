//! Scamper — a terminal 2D platformer engine that renders via the Kitty graphics
//! protocol. Local-first, single-player, keyboard-only. See PROJECT_PLAN.md.

pub mod dbg;
pub mod framebuffer;
pub mod input;
pub mod kitty;
pub mod math;
pub mod player;
pub mod png;
pub mod terminal;
pub mod time;
pub mod world;

pub use framebuffer::{Framebuffer, Rgba};
pub use math::{vec2, Vec2};

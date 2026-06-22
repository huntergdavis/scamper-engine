//! Scamper — a terminal 2D platformer engine that renders via the Kitty graphics
//! protocol. Local-first, single-player, keyboard-only. See PROJECT_PLAN.md.

pub mod framebuffer;
pub mod math;
pub mod png;

pub use framebuffer::{Framebuffer, Rgba};
pub use math::{vec2, Vec2};

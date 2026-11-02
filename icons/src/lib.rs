//! Renders inventory icons from Minecraft and mod JARs on the user's machine.
//!
//! Textures are not free to redistribute, so nothing this crate produces belongs in the
//! repository: it only holds the tool, every user generates the pictures locally.

pub mod assets;
pub mod entity;
pub mod fetch;
pub mod generate;
pub mod mesh;
pub mod model;
pub mod render;
pub mod texture;
pub mod tint;

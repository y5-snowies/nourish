//! # compositor_background_three_lock_scene
//!
//! Facade: sphere mesh that flattens to a plane during the "demorph"
//! animation. The scene's systems, material, mesh and state live in flat
//! sibling crates; this crate keeps `MorphScene` (and its `BevyScene` impl)
//! plus re-exports of every public item.

pub mod scene;
pub use scene::*;

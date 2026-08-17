//! `BevyRegistry`: the compositor-facing API for all Bevy scene instances
//! (method bodies live in bevy.lifecycle / bevy.order / bevy.frame / bevy.mutate).

pub mod base;
pub use base::*;

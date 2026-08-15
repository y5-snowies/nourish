//! Runtime shader loader: resolve a selection, try the active renderer's format
//! fallback order, compile the first that succeeds, and surface its declared
//! properties. Returns `None` (→ caller uses the built-in parallax) when no
//! format is present or none compiles. Every failure is logged, never fatal.

#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod base;
pub use base::*;

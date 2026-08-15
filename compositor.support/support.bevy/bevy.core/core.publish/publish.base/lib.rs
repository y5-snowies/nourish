//! The board the worker publishes onto and the compositor reads.
//!
//! One entry per live instance: the dmabuf whose write has COMPLETED, and the
//! generation it was published at. The compositor builds its render element from
//! this and nothing else — it never touches the worker's `App`, its wgpu textures
//! or the slot currently being drawn into.
//!
//! Read on the compositor thread once per instance per frame, written on the
//! worker once per publish, so an `RwLock` around a small map is the right shape.

pub mod base;
pub use base::*;

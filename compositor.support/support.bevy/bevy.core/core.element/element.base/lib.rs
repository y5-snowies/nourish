//! `BevyRenderElement`: the render element you add to your render list.
//! The Bevy app has already rendered into a dmabuf-backed wgpu texture; we
//! sample the corresponding GLES texture and composite it at screen coords.

pub mod base;
pub use base::*;

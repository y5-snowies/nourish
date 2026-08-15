//! Import a Smithay `Dmabuf` as a `wgpu::Texture`. Single-plane only —
//! ARGB8888 LINEAR is single-plane, so we're fine.

pub mod base;
pub use base::*;

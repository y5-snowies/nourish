//! One entry of the publish ring: a dmabuf imported as both a wgpu render
//! attachment (Bevy draws into it) and a `GlesTexture` (the compositor samples
//! it). Was the body of `BevySurface` before the surface became a ring of these.

pub mod base;
pub use base::*;

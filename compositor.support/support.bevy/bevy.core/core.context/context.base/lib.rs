//! WGPU Vulkan context with dmabuf-import features enabled. One per process for
//! the Bevy subsystem (Iced keeps its own — they don't share).

pub mod base;
pub use base::*;

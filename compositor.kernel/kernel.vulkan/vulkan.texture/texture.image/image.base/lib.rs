//! `VulkanTexture` — a reference-counted sampled image, extracted into its own
//! leaf crate so the renderer, the SHM texture cache, and the upload path can
//! all share the type without depending on `renderer.core`.

pub mod image;

pub use image::{RetiredTexture, TextureInner, VulkanTexture};

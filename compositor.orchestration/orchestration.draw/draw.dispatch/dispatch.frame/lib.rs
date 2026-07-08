//! `SceneDispatch` — the per-renderer dispatch seam that keeps scene elements
//! renderer-agnostic while letting each renderer supply its own implementation.
//!
//! A handful of scene elements (the iced UI surface, the bevy 3D background,
//! the GLES pixel-shader parallax background) are produced as GLES resources
//! (`GlesTexture`, `GlesPixelProgram`). Rather than weld those elements to
//! `GlesRenderer`, they stay generic over `R: SceneDispatch` and dispatch
//! through this trait (defined on the renderer, taking the frame as a param —
//! avoiding the GAT-HRTB limitation rust#100013):
//!
//! - `GlesRenderer` implements it for real (renders the texture / runs the
//!   pixel shader) — see the impl in this crate.
//! - `VulkanRenderer` (in the vulkan backend) and any other renderer implement
//!   it as a blank draw until their renderer-native path lands.

pub mod frame;
pub use frame::{ElementMeta, ElementSpace, NativeShaderPass, ParallaxUniforms, SceneDispatch, ShaderVariant};

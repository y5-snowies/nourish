//! Runtime WGSL/GLSL → SPIR-V compilation via naga (the Vulkan path). Validate,
//! then `write_vec(.., None)` so every entry point lands in one blob, Y-flip
//! disabled (geometry is authored in Vulkan clip space). WGSL yields one module
//! (`vs_main`+`fs_main`); a `glsl/` fragment or a composed multipass pass is
//! fragment-only and pairs with the prebuilt fullscreen-vertex module.

pub mod base;
pub use base::*;

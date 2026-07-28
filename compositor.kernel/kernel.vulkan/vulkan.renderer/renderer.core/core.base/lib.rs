//! `VulkanRenderer` — a Smithay `Renderer`/`Frame` implementation assembled
//! from the `backend.vulkan` piece-crates (device, memory import/export,
//! composite pipeline, command recording, element draws).
//!
//! Status (Phase 4, Step 0 — the "Path (a)" shim): this is the renderer-side
//! foundation. It implements the full Smithay renderer trait surface so a
//! `VulkanRenderer` is a drop-in wherever `R: Renderer` is expected. The GPU
//! command paths are wired to the real piece-crates but are **compile-verified
//! only** — they have not been exercised on hardware (no DRM/Vulkan session in
//! the build environment). The execution model is deliberately simple and
//! synchronous for the foundation (per-frame `device_wait_idle`, full-target
//! clear approximation, one reused command buffer); these are marked inline and
//! are the first things to refine once it runs on a GPU.
//!
//! It is NOT yet wired into the compositor scene path — that is the separate
//! "convergence" phase (generifying the GLES-typed `State.gpus` substrate).

#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod dispatch;
pub mod frame;
pub mod renderer;
mod shm_cache;

/// Generic native fullscreen-shader pass (the parallax background and any other
/// native fullscreen shader run through it). The specific shaders + push layouts
/// live in the scene layer; the kernel keeps only this generalization.
pub use compositor_kernel_vulkan_pipeline_fullscreen_base::fullscreen;
/// The HDR composite pipeline lives under `vulkan.pipeline`; re-export at the
/// historical `crate::hdr_composite` path.
pub use compositor_kernel_vulkan_pipeline_hdr_base::hdr as hdr_composite;

/// The `sync_file`/timeline `Fence` impls now live in `vulkan.sync/sync.fence`;
/// re-export at the historical `crate::sync_fence` path.
pub use compositor_kernel_vulkan_sync_fence_base::fence as sync_fence;

/// Re-export the renderer error type (now in the leaf `renderer.error` crate) at
/// the historical `crate::error` path so existing `crate::error::VulkanError`
/// references and the public `VulkanError` export keep resolving unchanged.
pub use compositor_kernel_vulkan_renderer_error_base::error;
pub use error::VulkanError;

/// Re-export `VulkanTexture`/`TextureInner` (now in the leaf `texture.image`
/// crate) at the historical `crate::texture` path so existing
/// `crate::texture::{VulkanTexture, TextureInner}` references keep resolving.
pub use compositor_kernel_vulkan_texture_image_base::image as texture;
pub use frame::{VulkanFrame, VulkanFramebuffer};
pub use renderer::VulkanRenderer;
pub use texture::VulkanTexture;

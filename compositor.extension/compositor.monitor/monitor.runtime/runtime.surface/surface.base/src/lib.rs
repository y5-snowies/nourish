//! # compositor_monitor_runtime_surface_base
//!
//! The transport layer for rendering GPU content into a shape that smithay's
//! GLES renderer can composite. Owns DMABUF allocation, WGPU/Vulkan device
//! creation, and the dmabuf↔wgpu↔GLES round-trip.
//!
//! Knows nothing about Iced. Sits underneath both `compositor_support_iced_core_engine_base`
//! (which renders into wgpu textures provided here) and `compositor_monitor_compositor_iced_base`
//! (which samples GLES textures provided here).
//!
//! ## Module layering
//!
//! ```text
//! surface.rs        IcedSurface  ─┬─ dmabuf_alloc.rs   AllocatedDmabuf
//!                                 ├─ wgpu_import.rs    Dmabuf -> wgpu::Texture
//!                                 ├─ gles_import.rs    Dmabuf -> GlesTexture
//!                                 └─ wgpu_context.rs   WgpuVulkanContext
//! error.rs          (used by all)
//! ```
//!
//! ## Typical use
//!
//! ```ignore
//! // Once at startup, ideally on a worker thread:
//! let ctx = create_wgpu_vulkan_context()?;
//! let ctx = ctx.into_arc();
//!
//! // Per Iced instance:
//! let surface = IcedSurface::allocate(&ctx, gles_renderer, size)?;
//! // surface.wgpu_texture  — render into this with iced_wgpu
//! // surface.gles_texture  — sample this in smithay's render pass
//! ```

#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod dmabuf_alloc;
pub mod error;
pub mod gles_import;
pub mod surface;
pub mod wgpu_context;
pub mod wgpu_import;

pub use dmabuf_alloc::{AllocatedDmabuf, allocate_dmabuf, allocate_dmabuf_on};
pub use error::{AllocError, GlesImportError, SurfaceError, WgpuContextError, WgpuImportError};
pub use gles_import::import_dmabuf_to_gles;
pub use surface::IcedSurface;
pub use wgpu_context::{WgpuVulkanContext, create_wgpu_vulkan_context};
pub use wgpu_import::{TEXTURE_FORMAT, TEXTURE_USAGE, import_dmabuf_to_wgpu};

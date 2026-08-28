//! # compositor_support_bevy_core_runtime_base
//!
//! Facade over the flat GPU-plumbing crates (dmabuf allocation, wgpu/Vulkan
//! context, GLES + wgpu imports). Every public path this crate historically
//! exposed keeps resolving through the re-exports below.

pub mod dmabuf_alloc {
    pub use compositor_support_bevy_core_alloc_base::{
        AllocatedDmabuf, allocate_dmabuf_negotiated,
    };
}
pub mod error {
    pub use compositor_support_bevy_core_fault_base::{
        AllocError, GlesImportError, SurfaceError, WgpuContextError, WgpuImportError,
    };
}
pub mod gles_import {
    pub use compositor_support_bevy_core_gles_base::import_dmabuf_to_gles;
}
pub mod surface {
    pub use compositor_support_bevy_core_surface_base::BevySurface;
}
pub mod wgpu_context {
    pub use compositor_support_bevy_core_context_base::{
        WgpuVulkanContext, create_wgpu_vulkan_context,
    };
}
pub mod wgpu_import {
    pub use compositor_support_bevy_core_import_base::{
        texture_format, TEXTURE_USAGE, import_dmabuf_to_wgpu,
    };
}

pub use dmabuf_alloc::{
    AllocatedDmabuf, allocate_dmabuf_negotiated,
};
pub use error::{AllocError, GlesImportError, SurfaceError, WgpuContextError, WgpuImportError};
pub use gles_import::import_dmabuf_to_gles;
pub use surface::BevySurface;
pub use wgpu_context::{WgpuVulkanContext, create_wgpu_vulkan_context};
pub use wgpu_import::{texture_format, TEXTURE_USAGE, import_dmabuf_to_wgpu};

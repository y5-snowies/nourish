//! Error types for the runtime layer.
//!
//! Every fallible operation in this crate funnels through one of these.

use thiserror;
use wgpu::Features;

#[derive(Debug, thiserror::Error)]
pub enum AllocError {
    #[error("failed to open DRM render node: {0}")]
    OpenDrm(std::io::Error),

    #[error("failed to initialize gbm device: {0}")]
    GbmInit(std::io::Error),

    #[error("failed to create gbm buffer object: {0}")]
    CreateBo(std::io::Error),

    #[error("failed to export fd for plane: {0}")]
    ExportFd(gbm::InvalidFdError),

    #[error("failed to build Dmabuf from gbm buffer")]
    BuildDmabuf,

    #[error("invalid dimensions: width and height must be > 0 (got {width}x{height})")]
    InvalidDimensions { width: u32, height: u32 },

    #[error("fourcc {0:?} has no gbm mapping for a linear scanout allocation")]
    UnsupportedFourcc(smithay::backend::allocator::Fourcc),
}

#[derive(Debug, thiserror::Error)]
pub enum WgpuContextError {
    #[error("no suitable Vulkan adapter found")]
    NoAdapter,

    #[error(
        "adapter doesn't support required features. required: {required:?}, supported: {supported:?}"
    )]
    MissingFeatures {
        required: Features,
        supported: Features,
    },

    #[error("failed to create Vulkan device: {0}")]
    DeviceCreation(wgpu::RequestDeviceError),
}

#[derive(Debug, thiserror::Error)]
pub enum WgpuImportError {
    #[error("adapter isn't using Vulkan backend (wgpu picked something else)")]
    NotVulkanBackend,

    #[error("dmabuf has {0} planes, but only single-plane is supported by this import path")]
    MultiPlaneNotSupported(usize),

    #[error("dmabuf has no fd")]
    NoFd,

    #[error("dmabuf has no stride")]
    NoStride,

    #[error("dmabuf has no offset")]
    NoOffset,

    #[error("failed to duplicate dmabuf fd: {0}")]
    FdDup(std::io::Error),

    #[error("wgpu-hal failed to import dmabuf: {0:?}")]
    HalImport(wgpu::hal::DeviceError),
}

#[derive(Debug, thiserror::Error)]
pub enum GlesImportError {
    #[error("GlesRenderer failed to import dmabuf: {0}")]
    ImportFailed(smithay::backend::renderer::gles::GlesError),
}

/// Aggregate error for `IcedSurface` operations (allocation + both imports).
#[derive(Debug, thiserror::Error)]
pub enum SurfaceError {
    #[error("dmabuf allocation: {0}")]
    Alloc(#[from] AllocError),

    #[error("wgpu import: {0}")]
    WgpuImport(#[from] WgpuImportError),

    #[error("gles import: {0}")]
    GlesImport(#[from] GlesImportError),
}

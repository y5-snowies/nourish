use thiserror;
use wgpu::Features;

#[derive(Debug, thiserror::Error)]
pub enum AllocError {
    #[error("invalid dimensions {width}x{height}: both must be non-zero")]
    InvalidDimensions { width: u32, height: u32 },

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

    #[error("dmabuf has {0} planes, but only single-plane is supported")]
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

#[derive(Debug, thiserror::Error)]
pub enum SurfaceError {
    #[error(transparent)]
    Alloc(#[from] AllocError),

    #[error(transparent)]
    GlesImport(#[from] GlesImportError),

    #[error(transparent)]
    WgpuImport(#[from] WgpuImportError),
}

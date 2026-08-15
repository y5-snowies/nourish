//! Bridge registry: entries that swap placeholder `GpuImage`s for
//! dmabuf-backed ones (installed by `BridgeRegistryPlugin`).

pub mod base;
pub use base::*;

//! `BridgeRegistryPlugin`: render-world systems that install pending
//! bridge entries (swap placeholder `GpuImage`s for dmabuf-backed ones).

pub mod base;
pub use base::*;

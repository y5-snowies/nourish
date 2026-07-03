//! Process-global diagnostics registry (Statistics tab data source) — façade.
//!
//! The implementation lives in the sibling crates (`registry.hdr` — live HDR encode
//! tuning, `registry.counter` — hot-path atomics, `registry.meta` — rare metadata,
//! `registry.snapshot` — the derived read side). This crate re-exports everything under
//! the `base` module so every existing `base::item` path (and `use ...::base as stats`)
//! keeps resolving unchanged.

pub mod base {
    pub use compositor_developer_stats_registry_counter::*;
    pub use compositor_developer_stats_registry_gpu::gpu::{DeviceFormat, set_device_format};
    pub use compositor_developer_stats_registry_hdr::*;
    pub use compositor_developer_stats_registry_meta::*;
    pub use compositor_developer_stats_registry_shader::*;
    pub use compositor_developer_stats_registry_snapshot::*;
}

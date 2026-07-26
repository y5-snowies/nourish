//! Post-determined GPU dmabuf formats, per device kind, for the developer tool's
//! "GPU formats" panel. Each publisher records the format+modifier ACTUALLY in use
//! the moment it's finalized (e.g. `bo.modifier()` after allocation); the stats
//! snapshot reads them. Mirrors `registry.meta` (a `Mutex` behind a `OnceLock`).

use std::sync::{Mutex, OnceLock};

/// One device's post-determined dmabuf format.
#[derive(Debug, Clone)]
pub struct DeviceFormat {
    /// Device kind, e.g. "gbm-bevy", "gbm-iced", "wgpu-bevy", "vulkan", "drm-scanout".
    pub kind: String,
    /// Fourcc name, e.g. "Argb8888".
    pub fourcc: String,
    /// Modifier as `0x…` hex.
    pub modifier: String,
    /// Class: "linear" | "tiled" | "tiled+dcc" | "invalid".
    pub class: String,
    /// DRM plane count of the chosen modifier.
    pub plane_count: u32,
    /// Whether a multi-plane (DCC/CCS) modifier is actually in use.
    pub multiplane: bool,
}

fn cell() -> &'static Mutex<Vec<DeviceFormat>> {
    static C: OnceLock<Mutex<Vec<DeviceFormat>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

/// Record (or replace) the post-determined format for `kind`. `class` should come
/// from the shared modifier classifier; `multiplane` is derived from `plane_count`.
pub fn set_device_format(kind: &str, fourcc: &str, modifier: u64, class: &str, plane_count: u32) {
    let df = DeviceFormat {
        kind: kind.to_string(),
        fourcc: fourcc.to_string(),
        modifier: format!("{modifier:#018x}"),
        class: class.to_string(),
        plane_count,
        multiplane: plane_count > 1,
    };
    let mut v = cell().lock().unwrap_or_else(|e| e.into_inner());
    match v.iter_mut().find(|d| d.kind == kind) {
        Some(slot) => *slot = df,
        None => v.push(df),
    }
}

/// Snapshot of every recorded device format (for the stats snapshot / gRPC).
pub fn device_formats() -> Vec<DeviceFormat> {
    cell().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

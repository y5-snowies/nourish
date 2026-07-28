//! Rare diagnostics metadata (renderer kind, sync mode, output/mode, VRR/HDR flags,
//! env): lives behind a mutex updated at setup/transition time.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub struct Meta {
    pub renderer: String,
    pub renderer_init_ok: bool,
    pub sync_mode: String,
    pub output_name: String,
    pub mode: String,
    pub vrr_supported: bool,
    pub vrr_enabled: bool,
    pub hdr_enabled: bool,
    /// Display advertises PQ HDR in EDID (independent of the active session path).
    pub hdr_capable: bool,
    /// Output transfer function in use ("SDR", "PQ", "HLG").
    pub hdr_transfer: String,
    /// Display max luminance from EDID (cd/m²), 0 if unknown.
    pub hdr_max_luminance: f32,
    /// Display advertises BT.2020 RGB colorimetry.
    pub hdr_bt2020: bool,
    /// Compositor working/scanout color format ("8-bit sRGB", "fp16→10-bit PQ").
    pub color_format: String,
    pub env_flags: Vec<(String, String)>,
    pub start: Instant,
    pub last_instant: Instant,
    pub last_frames: u64,
    pub last_vblanks: u64,
    pub last_fps: f32,
    pub last_vblank_rate: f32,
}

impl Default for Meta {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            renderer: "unknown".into(), renderer_init_ok: false, sync_mode: "unknown".into(),
            output_name: String::new(), mode: String::new(), vrr_supported: false,
            vrr_enabled: false, hdr_enabled: false, hdr_capable: false,
            hdr_transfer: "SDR".into(), hdr_max_luminance: 0.0, hdr_bt2020: false,
            color_format: "8-bit sRGB".into(), env_flags: Vec::new(), start: now,
            last_instant: now, last_frames: 0, last_vblanks: 0, last_fps: 0.0,
            last_vblank_rate: 0.0,
        }
    }
}

pub fn meta() -> &'static Mutex<Meta> {
    static META: OnceLock<Mutex<Meta>> = OnceLock::new();
    META.get_or_init(|| Mutex::new(Meta::default()))
}

pub fn set_renderer(kind: &str, init_ok: bool) {
    let mut m = meta().lock().unwrap_or_else(|e| e.into_inner());
    m.renderer = kind.to_owned();
    m.renderer_init_ok = init_ok;
}

pub fn set_sync_mode(mode: &str) {
    meta().lock().unwrap_or_else(|e| e.into_inner()).sync_mode = mode.to_owned();
}

pub fn set_output(name: &str, mode: &str) {
    let mut m = meta().lock().unwrap_or_else(|e| e.into_inner());
    m.output_name = name.to_owned();
    m.mode = mode.to_owned();
}

pub fn set_vrr(supported: bool, enabled: bool) {
    let mut m = meta().lock().unwrap_or_else(|e| e.into_inner());
    m.vrr_supported = supported;
    m.vrr_enabled = enabled;
}

pub fn set_hdr(enabled: bool) {
    meta().lock().unwrap_or_else(|e| e.into_inner()).hdr_enabled = enabled;
}

/// Full HDR/color state for the Statistics tab: `active` = HDR output path on this
/// session; `capable` = display advertises PQ in EDID; `transfer` = output transfer
/// function; `max_luminance` cd/m² (0=unknown); `bt2020` = wide-gamut colorimetry.
#[allow(clippy::too_many_arguments)]
pub fn set_hdr_info(
    active: bool, capable: bool, transfer: &str,
    max_luminance: f32, bt2020: bool, color_format: &str,
) {
    let mut m = meta().lock().unwrap_or_else(|e| e.into_inner());
    m.hdr_enabled = active;
    m.hdr_capable = capable;
    m.hdr_transfer = transfer.to_owned();
    m.hdr_max_luminance = max_luminance;
    m.hdr_bt2020 = bt2020;
    m.color_format = color_format.to_owned();
}

pub fn set_env_flags(flags: Vec<(String, String)>) {
    meta().lock().unwrap_or_else(|e| e.into_inner()).env_flags = flags;
}

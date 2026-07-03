//! Read-side of the diagnostics registry: `snapshot()` reads the counters + metadata and
//! derives FPS / vblank-rate over the interval since the last snapshot. The developer-log
//! gRPC service calls it on demand (the Statistics tab's Refresh button).

use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

use compositor_developer_stats_registry_counter::{
    FENCE_FALLBACK, FENCE_KMS_INFENCE, FENCE_SYNCHRONOUS, FRAMES, VBLANKS,
};
use compositor_developer_stats_registry_meta::meta;
pub use compositor_developer_stats_registry_gpu::gpu::DeviceFormat;

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub renderer: String,
    pub renderer_init_ok: bool,
    pub sync_mode: String,
    pub output_name: String,
    pub mode: String,
    pub vrr_supported: bool,
    pub vrr_enabled: bool,
    pub hdr_enabled: bool,
    pub hdr_capable: bool,
    pub hdr_transfer: String,
    pub hdr_max_luminance: f32,
    pub hdr_bt2020: bool,
    pub color_format: String,
    pub frames_total: u64,
    pub vblanks_total: u64,
    pub fps: f32,
    pub vblank_rate: f32,
    pub fence_synchronous: u64,
    pub fence_kms_infence: u64,
    pub fence_fallback: u64,
    pub env_flags: Vec<(String, String)>,
    pub uptime_secs: f64,
    /// Post-determined per-device dmabuf formats (bridge/scanout), for the
    /// "GPU formats" panel.
    pub device_formats: Vec<DeviceFormat>,
}

/// Read all stats and derive FPS / vblank-rate over the interval since the last
/// `snapshot()` call (so successive refreshes show live rates).
pub fn snapshot() -> Snapshot {
    let frames_total = FRAMES.load(Relaxed);
    let vblanks_total = VBLANKS.load(Relaxed);
    let mut m = meta().lock().unwrap_or_else(|e| e.into_inner());

    let now = Instant::now();
    let dt = now.duration_since(m.last_instant).as_secs_f32();
    // Only recompute rates over a meaningful interval; otherwise keep the last
    // values (avoids divide-by-tiny spikes on rapid refreshes).
    if dt >= 0.1 {
        m.last_fps = frames_total.saturating_sub(m.last_frames) as f32 / dt;
        m.last_vblank_rate = vblanks_total.saturating_sub(m.last_vblanks) as f32 / dt;
        m.last_instant = now;
        m.last_frames = frames_total;
        m.last_vblanks = vblanks_total;
    }

    Snapshot {
        renderer: m.renderer.clone(),
        renderer_init_ok: m.renderer_init_ok,
        sync_mode: m.sync_mode.clone(),
        output_name: m.output_name.clone(),
        mode: m.mode.clone(),
        vrr_supported: m.vrr_supported,
        vrr_enabled: m.vrr_enabled,
        hdr_enabled: m.hdr_enabled,
        hdr_capable: m.hdr_capable,
        hdr_transfer: m.hdr_transfer.clone(),
        hdr_max_luminance: m.hdr_max_luminance,
        hdr_bt2020: m.hdr_bt2020,
        color_format: m.color_format.clone(),
        frames_total,
        vblanks_total,
        fps: m.last_fps,
        vblank_rate: m.last_vblank_rate,
        fence_synchronous: FENCE_SYNCHRONOUS.load(Relaxed),
        fence_kms_infence: FENCE_KMS_INFENCE.load(Relaxed),
        fence_fallback: FENCE_FALLBACK.load(Relaxed),
        env_flags: m.env_flags.clone(),
        uptime_secs: now.duration_since(m.start).as_secs_f64(),
        device_formats: compositor_developer_stats_registry_gpu::gpu::device_formats(),
    }
}

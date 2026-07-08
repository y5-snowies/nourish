//! Hot-path diagnostics counters: plain relaxed atomics — a per-frame increment is a few
//! ns, negligible. The render/backend paths call the cheap update fns; `snapshot()` (in
//! the sibling snapshot crate) reads the raw statics.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

/// Window over which the per-output present rate is averaged and then reset.
const PRESENT_WINDOW_SECS: f64 = 0.5;

/// Per-output presented-frame meter. Counts real page-flip completions (native)
/// / presents (winit) over a short window and stores the derived rate, then
/// RESETS the count — no ever-growing counter. Because it counts what actually
/// reached the screen, a dropped frame (a vblank with no new buffer) shows up as
/// a lower rate rather than being masked. Read by the FPS overlay.
struct PresentMeter {
    frames: u32,
    last: Instant,
    rate: u32,
}

/// Keyed by output key. Lock is uncontended (one writer per pipe at vblank, one
/// reader per composited frame); `get_mut` avoids allocating on the steady path.
static PRESENTS: LazyLock<Mutex<HashMap<String, PresentMeter>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record one presented frame on `output_key`; recompute + reset the windowed
/// rate when the window elapses. Call at page-flip completion (native) / present
/// (winit).
pub fn present(output_key: &str) {
    let now = Instant::now();
    let mut map = PRESENTS.lock().unwrap();
    if let Some(m) = map.get_mut(output_key) {
        m.frames += 1;
        let dt = now.duration_since(m.last).as_secs_f64();
        if dt >= PRESENT_WINDOW_SECS {
            m.rate = (m.frames as f64 / dt).round() as u32;
            m.frames = 0;
            m.last = now;
        }
    } else {
        map.insert(
            output_key.to_string(),
            PresentMeter {
                frames: 1,
                last: now,
                rate: 0,
            },
        );
    }
}

/// The last windowed present rate (fps) on `output_key`, 0 if unknown.
pub fn present_rate(output_key: &str) -> u32 {
    PRESENTS.lock().unwrap().get(output_key).map(|m| m.rate).unwrap_or(0)
}

pub static FRAMES: AtomicU64 = AtomicU64::new(0);
pub static VBLANKS: AtomicU64 = AtomicU64::new(0);
pub static FENCE_SYNCHRONOUS: AtomicU64 = AtomicU64::new(0);
pub static FENCE_KMS_INFENCE: AtomicU64 = AtomicU64::new(0);
pub static FENCE_FALLBACK: AtomicU64 = AtomicU64::new(0);
/// The active compositor renderer composites from dmabufs (Vulkan), set once the
/// renderer is chosen (after any GLES fallback). Producers/capture use it to skip
/// building GLES-path-only resources that the Vulkan compositor never samples.
/// Defaults to `false` (GLES) so nothing is skipped before it's set.
pub static COMPOSITOR_PREFERS_DMABUF: AtomicBool = AtomicBool::new(false);

/// One composited frame. Call once per present.
#[inline]
pub fn frame() {
    FRAMES.fetch_add(1, Relaxed);
}

/// One vblank / page-flip completion. Call from frame_submitted.
#[inline]
pub fn vblank() {
    VBLANKS.fetch_add(1, Relaxed);
}

/// The renderer used a synchronous (device_wait_idle) completion this frame.
#[inline]
pub fn fence_synchronous() {
    FENCE_SYNCHRONOUS.fetch_add(1, Relaxed);
}

/// The renderer produced a KMS IN_FENCE (DRM syncobj sync_file) this frame.
#[inline]
pub fn fence_kms_infence() {
    FENCE_KMS_INFENCE.fetch_add(1, Relaxed);
}

/// A modern-sync attempt failed and fell back to a device drain this frame.
#[inline]
pub fn fence_fallback() {
    FENCE_FALLBACK.fetch_add(1, Relaxed);
}

/// Live world-camera zoom (`f64` via `to_bits`), pushed from the one camera
/// `SetZoom` write point and read every frame by the Vulkan renderer to
/// zoom-weight the anti-aliasing knobs (world anti-aliasing graphics config). `1.0` == 100%.
pub static WORLD_CAMERA_ZOOM: AtomicU64 = AtomicU64::new(0x3FF0_0000_0000_0000); // 1.0f64

/// Record whether the active compositor renderer composites from dmabufs
/// (Vulkan). Call once after the renderer (and any fallback) is resolved.
#[inline]
pub fn set_compositor_prefers_dmabuf(yes: bool) {
    COMPOSITOR_PREFERS_DMABUF.store(yes, Relaxed);
}

/// Publish the current world-camera zoom (see [`WORLD_CAMERA_ZOOM`]). Called
/// from the camera `SetZoom` path.
#[inline]
pub fn set_world_zoom(zoom: f64) {
    WORLD_CAMERA_ZOOM.store(zoom.to_bits(), Relaxed);
}

/// Current world-camera zoom (`1.0` == 100%).
#[inline]
pub fn world_zoom() -> f64 {
    f64::from_bits(WORLD_CAMERA_ZOOM.load(Relaxed))
}

/// True if the active compositor renderer composites from dmabufs (Vulkan), so
/// GLES-path-only resources can be skipped. See [`set_compositor_prefers_dmabuf`].
#[inline]
pub fn compositor_prefers_dmabuf() -> bool {
    COMPOSITOR_PREFERS_DMABUF.load(Relaxed)
}

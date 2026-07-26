//! Live HDR encode parameters (M5), set from the developer tool over gRPC and read by
//! the renderer each frame into the encode shader's uniform. A version counter lets the
//! renderer re-upload the UBO only when something changed. Field order + types mirror
//! the WGSL `Tuning` struct exactly (all f32, tightly packed).

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

/// Live-tunable HDR encode parameters. Mirrors the WGSL `Tuning` uniform.
#[derive(Debug, Clone, Copy)]
pub struct HdrTuning {
    /// 0 = passthrough (SDR look, no conversion), 1 = HDR encode active.
    pub enabled: f32,
    /// Luminance (cd/m²) that SDR white maps to (PQ reference ≈ 203).
    pub sdr_white_nits: f32,
    /// Display max luminance (cd/m²) — tone-map target.
    pub max_nits: f32,
    /// Overall linear gain (1 = none).
    pub brightness: f32,
    /// Contrast around mid-grey (1 = none).
    pub contrast: f32,
    /// Saturation (1 = none, 0 = greyscale, >1 = boosted).
    pub saturation: f32,
    /// Rec.709→BT.2020 primaries mix (0 = leave, 1 = full).
    pub gamut: f32,
    /// 0 = hard clip highlights, 1 = Reinhard rolloff.
    pub tone_map: f32,
    /// Output transfer: 0 = PQ (ST 2084), 1 = HLG.
    pub transfer: f32,
    /// Extra power on linear (1 = none).
    pub gamma: f32,
    /// Extra linear multiplier (1 = none).
    pub exposure: f32,
}

impl Default for HdrTuning {
    fn default() -> Self {
        Self {
            enabled: 1.0,
            sdr_white_nits: 203.0,
            max_nits: 1000.0,
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            gamut: 1.0,
            tone_map: 1.0,
            transfer: 0.0,
            gamma: 1.0,
            exposure: 1.0,
        }
    }
}

static HDR_TUNING_VERSION: AtomicU64 = AtomicU64::new(0);

fn hdr_tuning_cell() -> &'static Mutex<HdrTuning> {
    static T: OnceLock<Mutex<HdrTuning>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HdrTuning::default()))
}

/// Replace the live HDR tuning (developer tool → gRPC). Bumps the version.
pub fn set_hdr_tuning(t: HdrTuning) {
    *hdr_tuning_cell().lock().unwrap_or_else(|e| e.into_inner()) = t;
    HDR_TUNING_VERSION.fetch_add(1, Relaxed);
}

/// Current HDR tuning (renderer reads this into the encode UBO).
pub fn hdr_tuning() -> HdrTuning {
    *hdr_tuning_cell().lock().unwrap_or_else(|e| e.into_inner())
}

/// Monotonic version; the renderer re-uploads the UBO only when it changes.
pub fn hdr_tuning_version() -> u64 {
    HDR_TUNING_VERSION.load(Relaxed)
}

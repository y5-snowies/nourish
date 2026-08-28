//! Mode selection policy (ex wire.rs): area -> refresh -> PREFERRED,
//! lexicographic. Consumes preference values when a profile requests an
//! advertised mode; otherwise unchanged default policy.

use compositor_kernel_graphic_preference_output_profile::profile::{ModeRequest, OutputProfile};
use smithay::reexports::drm::control::{connector, Mode as DrmMode, ModeFlags, ModeTypeFlags};

/// Whether `m` is interlaced. An interlaced mode must never be chosen over a
/// progressive one: the scanout planes reject tiled/CCS buffers on an interlaced
/// CRTC, so every explicit-modifier format fails the atomic test. smithay reacts
/// to that by falling the WHOLE DEVICE back to implicit modifiers — and the
/// Vulkan renderer cannot create images for implicit-modifier buffers at all, so
/// the result is not a degraded pipe but a blank screen on every output.
/// Observed live: a 1280x1024 panel advertising `1920x1080i` took the whole
/// compositor dark once it was plugged in as a second monitor.
pub fn is_interlaced(m: &DrmMode) -> bool {
    m.flags().contains(ModeFlags::INTERLACE)
}

/// Default policy: progressive first, then area -> refresh -> PREFERRED.
///
/// `!is_interlaced` is the HIGHEST-priority key, ahead of area — an interlaced
/// mode is only ever selected when the connector advertises nothing else.
pub fn select_default(info: &connector::Info) -> Option<DrmMode> {
    info.modes()
        .iter()
        .max_by_key(|m| {
            let (w, h) = m.size();
            let area = (w as u64) * (h as u64);
            let refresh = m.vrefresh();
            let is_preferred = m.mode_type().contains(ModeTypeFlags::PREFERRED);
            (!is_interlaced(m), area, refresh, is_preferred)
        })
        .copied()
}

/// Preference-aware selection: an `Advertised` request narrows the list; any
/// synthesis request is NOT handled here (that is `mode.synthesize`, Law 7).
pub fn select(info: &connector::Info, profile: Option<&OutputProfile>) -> Option<DrmMode> {
    if let Some(OutputProfile { mode: Some(ModeRequest::Advertised { width, height, refresh_mhz }), .. }) = profile {
        // A requested WxH@R can be advertised both progressive and interlaced
        // (1920x1080 vs 1920x1080i). Match the progressive one first — see
        // `is_interlaced` for why taking the interlaced variant blanks the device.
        let matches = |m: &&DrmMode| {
            let (w, h) = m.size();
            w == *width && h == *height && m.vrefresh() * 1000 == *refresh_mhz
        };
        let hit = info
            .modes()
            .iter()
            .find(|m| matches(m) && !is_interlaced(m))
            .or_else(|| info.modes().iter().find(matches));
        if let Some(m) = hit {
            return Some(*m);
        }
        warn!("requested advertised mode not found; falling back to default policy");
    }
    select_default(info)
}

pub fn log_selected(mode: &DrmMode) {
    info!(
        "selected mode: {}x{} @ {}Hz, type: {:?}",
        mode.size().0,
        mode.size().1,
        mode.vrefresh(),
        mode.mode_type(),
    );
}

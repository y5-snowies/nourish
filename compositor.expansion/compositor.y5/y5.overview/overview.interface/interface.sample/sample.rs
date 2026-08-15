//! Ask the background sampler for a fresh pass over the windows of the world
//! the overview is opening on.
//!
//! The sampler's own cadence visits each window every 30–60s (see
//! `sampler.window/window.schedule`), which is right for a background refresh
//! and far too slow for an overlay the user just opened. So opening the overview
//! requests one out-of-cadence pass over every window on the active world.
//!
//! It is a REQUEST, not a wait, and the sampler grants it only for windows whose
//! last sample is older than its cadence floor — so reopening the overview
//! repeatedly is free, and the overlay simply draws from the latest sample it
//! already has until a fresh one replaces it. What this call
//! contributes that the sampler thread cannot get for itself is the wayland half
//! of each window's identity — title, app_id, credentials and the
//! `xdg_toplevel_icon_v1` name — which it hands over with the request.

use compositor_introspection_extraction_window_base::{extract_surface_meta, Meta};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_driver_introspection_base::base::SAMPLER;
use compositor_y5_window_interface_record::window::LoopWindow;
use uuid::Uuid;

pub fn request(state: &mut Loop) {
    // Cloned so the surface walk below doesn't hold a borrow of `inner`.
    let display_handle = state.inner.loader.display_handle.clone();
    let surfaces: Vec<(Uuid, Meta)> = state
        .inner
        .space_state()
        .state
        .elements()
        .filter_map(|window| {
            let uuid = window.uuid()?;
            // Surface-only: no /proc walk on the main thread — that is the whole
            // point of handing the work to the sampler.
            Some((uuid, extract_surface_meta(window, &display_handle)?))
        })
        .collect();
    if let Some(sampler) = state.inner.kernel.get(&SAMPLER) {
        sampler.request_immediate(surfaces);
    }
}

//! Stamp the tearing "target" tag on a newly-mapped window.
//!
//! Resolved ONCE, at map time. The heuristic behind it walks `/proc` and the
//! process tree, which is far too heavy for `commit` — and `commit` is exactly
//! where the tag is read, on every client buffer.
//!
//! The marker is the same one `wp_tearing_control_v1` writes, so a client that
//! does speak the protocol still has the last word: its hint may arrive at any
//! time and toggles the flag either way. Only a positive result is written here —
//! a client that already asked for tearing before mapping is not un-tagged by a
//! heuristic that merely failed to recognise it.

use compositor_support_smithay_state_tearing_pacer::pacer::PacerSurface;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_record::window::LoopWindow;
use smithay::desktop::Window;
use smithay::wayland::seat::WaylandFocus;

pub fn tag(state: &Loop, window: &Window) {
    let cfg = compositor_model_environment_tearing_config::config::get();
    let space = &state.inner.space_state().state;
    let dh = &state.inner.loader.display_handle;
    let Some(node) = window.meta(space, dh) else { return };
    if !compositor_y5_graphic_tearing_heuristic::heuristic::is_target(&node, cfg.tag.steam) {
        trace!(
            "tearing: not a target (app_id={:?} exe={:?} steam_rule={})",
            node.meta.app_id, node.meta.exe, cfg.tag.steam
        );
        return;
    }
    let Some(surface) = window.wl_surface() else { return };
    smithay::wayland::compositor::with_states(&surface, |states| {
        states.data_map.insert_if_missing(|| PacerSurface::new(true));
        if let Some(t) = states.data_map.get::<PacerSurface>() {
            t.set(true);
        }
    });
    info!(
        "tearing: auto-tagged window as target (app_id={:?} exe={:?})",
        node.meta.app_id, node.meta.exe
    );
}

//! Stamp the tearing "target" tag on a newly-mapped window.
//!
//! Resolved ONCE, at map time. The heuristic behind it walks `/proc` and the
//! process tree, which is far too heavy for `commit` — and `commit` is exactly
//! where the tag is read, on every client buffer.
//!
//! What is written is the COMPOSITOR's half of `pacer::TearingTag`; the client writes
//! the other half through `wp_tearing_control_v1` and the two never touch. Sharing one
//! flag was the bug: the protocol rewrites its own on every `set_presentation_hint`, so
//! a game that asked for vsync erased this verdict once per frame.
//!
//! The half written depends on WHO decided. `Y5_TEARING=1` is the user, about this exact
//! process, and outranks the client's own hint; Steam attribution is a guess, and the
//! client's hint outranks it. `pacer::Verdict::is_target` holds that ladder.
//!
//! Only a positive result is written, so a heuristic that merely failed to recognise a
//! client cannot un-tag one.

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_record::window::LoopWindow;
use smithay::desktop::Window;
use smithay::wayland::seat::WaylandFocus;

pub fn tag(state: &Loop, window: &Window) {
    let cfg = compositor_model_environment_tearing_config::config::get();
    let space = &state.inner.space_state().state;
    let dh = &state.inner.loader.display_handle;
    let Some(node) = window.meta(space, dh) else { return };
    use compositor_y5_graphic_tearing_heuristic::heuristic::{classify, Source};
    let Some(source) = classify(&node, cfg.tag.steam) else { return };
    // Through the facade, which survives an X11 window that has no `wl_surface` yet:
    // Xwayland associates one only after the window exists, so the map request can arrive
    // first. The verdict is resolved once and never revisited, so a stamp with nowhere to
    // go would be lost for good.
    compositor_support_smithay_state_window_shell::shell::mark_tearing_target(
        window,
        source == Source::Forced,
    );
}

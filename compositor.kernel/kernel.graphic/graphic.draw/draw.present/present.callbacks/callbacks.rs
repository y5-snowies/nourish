//! Presentation-feedback collection, frame callbacks, and post-frame
//! housekeeping. Replaces the `refresh()` blocks duplicated in both backends.

use smithay::desktop::utils::OutputPresentationFeedback;
use smithay::desktop::{layer_map_for_output, Window};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use std::time::Duration;
use compositor_orchestration_core_state_base::Loop;

/// The presentation Kind flags this compositor reports for a hardware flip.
///
/// `Vsync` asserts the presentation was synchronized to the vertical retrace, so
/// it MUST be dropped for a frame that was async-flipped — clients (Mesa's WSI,
/// engine frame pacers) read this flag to detect tearing and adapt their own
/// pacing, and reporting it on a torn frame feeds them a false signal.
///
/// `HwClock` and `HwCompletion` stay set for BOTH, and deliberately. Neither
/// makes a claim about retrace: they say the timestamp came from the display
/// hardware and that the hardware signalled the presentation, and an async flip
/// satisfies both — the kernel's page-flip event IS the hardware saying the new
/// buffer began scanning out. Dropping them on a torn frame would replace one
/// true statement with a vaguer one; `Vsync` is the only flag that was ever
/// wrong here, and the truthful per-frame mix is what the protocol asks for.
/// The other half of the same honesty is `Refresh::Unknown` at `wire.frame`:
/// a torn frame genuinely has no predictable next presentation.
pub fn hw_flip_kind(tearing: bool) -> wp_presentation_feedback::Kind {
    let hw = wp_presentation_feedback::Kind::HwClock | wp_presentation_feedback::Kind::HwCompletion;
    if tearing { hw } else { wp_presentation_feedback::Kind::Vsync | hw }
}

/// Collect presentation feedback for the windows visible in the frame that is
/// about to be queued. (Ex udev `refresh()` first half.)
pub fn collect_feedback(output: &Output, visible: &[Window]) -> OutputPresentationFeedback {
    let mut feedback = OutputPresentationFeedback::new(output);
    for window in visible {
        window.take_presentation_feedback(
            &mut feedback,
            |_, _| Some(output.clone()),
            // Collection-time placeholder; the real flags are chosen at
            // `presented()`, once the flip mode for this frame is known.
            |_, _| hw_flip_kind(false),
        );
    }
    feedback
}

/// Send frame callbacks to the visible windows. `throttle` follows the
/// caller's existing behavior (`Some(Duration::ZERO)` in both backends today).
pub fn send_window_frames(state: &Loop, output: &Output, visible: &[Window]) {
    let frame_time = state.inner.start_time.elapsed();
    for window in visible {
        window.send_frame(output, frame_time, Some(Duration::ZERO), |_, _| {
            Some(output.clone())
        });
    }
}

/// Send frame callbacks to the layers of ONE output — the one that just
/// presented. Multi-output: firing every output's layers on any output's flip
/// would pace a slow monitor's bar/panel at a fast neighbour's refresh; layer
/// surfaces are per-output (smithay keys layer maps by `Output`), so each
/// output drives only its own layers on its own vblank.
pub fn send_layer_frames(state: &Loop, output: &Output) {
    let frame_time = state.inner.start_time.elapsed();
    let layer_map = layer_map_for_output(output);
    for layer in layer_map.layers() {
        layer.send_frame(output, frame_time, None, |_surface, _states| {
            Some(output.clone())
        });
    }
}

/// Post-frame housekeeping: space refresh, popup cleanup, client flush.
/// Runs every frame, damage or no damage.
///
/// `refresh_space` rather than smithay's `Space::refresh()`: the latter derives
/// `wl_output` enter/leave from the window's stored position, which in y5 is a WORLD
/// coordinate and not a screen one — and no window belongs to one output anyway, since
/// every monitor renders the same world through its own camera.
pub fn housekeeping(state: &mut Loop) {
    state.inner.refresh_space();
    state.state.popup.state.cleanup();
    let _ = state.inner.loader.display_handle.flush_clients();
}

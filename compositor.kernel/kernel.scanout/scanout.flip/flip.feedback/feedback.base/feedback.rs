//! frame_submitted pop + presented() with the compositor's Kind flags.

use compositor_kernel_scanout_surface_output_base::output::NativeDrmOutput;
use smithay::desktop::utils::OutputPresentationFeedback;
use smithay::utils::Monotonic;
use smithay::wayland::presentation::Refresh;
use std::time::Duration;

/// Pop the user data for the frame that just hit the screen.
/// Outer Option: was there a submitted frame at all; inner: its user data.
pub fn pop(output: &mut NativeDrmOutput) -> Option<Option<OutputPresentationFeedback>> {
    match output.frame_submitted() {
        Ok(Some(feedback)) => {
            // A frame reached the screen — count the vblank/page-flip.
            compositor_model_stats_registry_base::base::vblank();
            Some(feedback)
        }
        Ok(None) => None,
        Err(err) => {
            warn!("frame_submitted error: {:?}", err);
            None
        }
    }
}

/// Fire presentation callbacks for the completed frame.
pub fn presented(
    feedback: &mut OutputPresentationFeedback,
    time: Duration,
    refresh: Refresh,
    sequence: u64,
    flags: smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind,
) {
    feedback.presented::<Duration, Monotonic>(time, refresh, sequence, flags);
}

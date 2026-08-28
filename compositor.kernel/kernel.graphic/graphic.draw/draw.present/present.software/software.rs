//! Presentation feedback for a backend with NO hardware page-flip — the nested
//! (winit) path. The native counterpart lives in `present.callbacks`
//! (`hw_flip_kind`) and is driven by the kernel's vblank event; nested there is no
//! event to wait for, so the frame is reported at submit from a software clock.

use smithay::desktop::utils::OutputPresentationFeedback;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::utils::{Clock, Monotonic};
use smithay::wayland::presentation::Refresh;
use std::time::Duration;

/// The Kind flags for a presentation performed without a page-flip.
///
/// `Vsync` is honest: the nested submit is paced by the HOST compositor's retrace, so
/// the frame really was synchronized to one. `HwClock` and `HwCompletion` are NOT, and
/// that is the whole reason these flags differ from `hw_flip_kind`: the timestamp comes
/// from our own `clock_gettime` rather than from display hardware, and nothing told us
/// the buffer began scanning out — the host may composite it later, or drop it. Setting
/// them would dress a software estimate up as a hardware measurement, which is exactly
/// the distinction these flags exist to let a frame pacer make.
pub fn software_present_kind() -> wp_presentation_feedback::Kind {
    wp_presentation_feedback::Kind::Vsync
}

/// Mark a collected feedback presented NOW.
///
/// Not reporting at all is the WORSE answer, and was the previous behaviour: the
/// `wp_presentation` global is advertised on both backends, so clients request feedback
/// either way, and a feedback destroyed without an answer reaches the client as
/// `discarded` — "your content never reached the screen". For a frame that did reach the
/// screen that is not a missing reply, it is a false one.
///
/// CLOCK_MONOTONIC because that is what the compositor advertises in
/// `state.presentation/presentation.factory` (`PresentationState::new(dh, 1)`); a
/// timestamp from another clock would be silently misread by every client.
///
/// The msc sequence is 0: nested has no vblank counter, and inventing a frame counter
/// would claim a hardware meaning it does not have.
pub fn presented_now(feedback: &mut OutputPresentationFeedback, output: &Output) {
    let now: Duration = Clock::<Monotonic>::new().now().into();
    let refresh = match output.current_mode() {
        // `refresh` is in mHz; a mode reporting 0 is "unknown", not "infinitely fast".
        Some(mode) if mode.refresh > 0 => {
            Refresh::Fixed(Duration::from_nanos(1_000_000_000_000u64 / mode.refresh as u64))
        }
        _ => Refresh::Unknown,
    };
    feedback.presented::<Duration, Monotonic>(now, refresh, 0, software_present_kind());
}

//! The exclusive-pacing floor watchdog: registered only while the redraw gate is
//! engaged.
//!
//! Under exclusivity the flip cadence belongs to the tagged client, so a client
//! that stops committing would otherwise stop the compositor with it — a loading
//! screen or a shader hitch freezing the desktop, cursor and UI included. This
//! guarantees a frame whenever a pipe has gone longer than its own measured
//! cadence without producing one (see `tearing.liveness`).
//!
//! Armed on the transition INTO engagement and dropped on the way out, rather
//! than run for the whole session: outside engagement nothing is gated, every
//! source schedules normally, and a 30Hz timer asking "has anything composited"
//! is answering a question no one asked.

use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_state_tearing_floor::floor;
use compositor_support_smithay_state_tearing_liveness::liveness;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{LoopHandle, RegistrationToken};

/// Register the watchdog if it is not already running. Idempotent.
pub fn arm(loop_handle: &LoopHandle<'static, Loop>, slot: &mut Option<RegistrationToken>) {
    if slot.is_some() {
        return;
    }
    // A repeating source rather than a per-frame timer: at 300fps the latter
    // would churn 300 registrations a second to answer a question that only
    // needs asking 30 times.
    match loop_handle.insert_source(Timer::from_duration(floor::poll()), |_, _, state: &mut Loop| {
        if compositor_support_smithay_state_tearing_gate::gate::engaged() && liveness::stalled() {
            state.force_redraw();
        }
        TimeoutAction::ToDuration(floor::poll())
    }) {
        Ok(token) => {
            *slot = Some(token);
            info!("tearing: pacing floor watchdog armed");
        }
        // Not fatal, but say so: without it a stalled pacer freezes the desktop
        // until unrelated input schedules a redraw.
        Err(e) => warn!("tearing: pacing floor watchdog registration failed: {e}"),
    }
}

/// Drop the watchdog and forget the measured cadences, so the next engagement
/// starts from its own pacer rather than inheriting the previous one's rate.
pub fn disarm(loop_handle: &LoopHandle<'static, Loop>, slot: &mut Option<RegistrationToken>) {
    if let Some(token) = slot.take() {
        loop_handle.remove(token);
        liveness::reset();
        info!("tearing: pacing floor watchdog disarmed");
    }
}

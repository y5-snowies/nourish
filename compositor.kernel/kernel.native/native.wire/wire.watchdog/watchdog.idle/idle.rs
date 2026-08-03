//! The last-resort unfreeze: a rescue that drives no cadence.
//!
//! The redraw loop is sustained by flip -> vblank -> render. Every producer that
//! can stall it has its own rescue — the background worker's wall-clock override
//! in `worker.pace`, the publish-wake in `frame.base` — but each of those only
//! covers the stall it knows about. This covers the case none of them can: the
//! loop stopped for a reason nothing is watching for.
//!
//! It CANNOT influence pacing, and that is the design constraint rather than a
//! happy accident. It acts only when the vblank counter has not moved for a full
//! [`IDLE_WINDOW`] — by definition, when nothing at all is driving frames. Any
//! flip from any source resets it, so at any real frame rate it observes and does
//! nothing. There is no rate here to interact with anything else's.
//!
//! Deliberately NOT configurable. A safety net whose failure mode is a frozen
//! desktop is not one to offer people a switch for, and at one wakeup per ten
//! seconds there is nothing to tune away from.

use compositor_orchestration_core_state_base::state::StatusSession;
use compositor_orchestration_core_state_base::Loop;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use std::sync::atomic::Ordering;
use std::time::Duration;

/// How long the compositor must produce NO flip before a frame is forced, and
/// equally the spacing between retries while it stays stuck.
///
/// Long on purpose. Short enough that a freeze is a hitch rather than a hang,
/// far too long to participate in anything: even the slowest preset renders a
/// hundred times inside one window.
pub const IDLE_WINDOW: Duration = Duration::from_secs(10);

/// `full_damage` is injected rather than reached for, following the same
/// pattern as `lifecycle.resume`'s steps: it lets this crate stay free of the
/// native context type while still being able to guarantee a flip.
pub fn arm(loop_handle: &LoopHandle<'static, Loop>, mut full_damage: impl FnMut() + 'static) {
    // `u64::MAX` so the first tick only ever OBSERVES: a rescue on startup, before
    // anything has had reason to flip, would be a false positive every boot.
    let mut last_vblanks = u64::MAX;
    let mut announced = false;
    let source = Timer::from_duration(IDLE_WINDOW);
    match loop_handle.insert_source(source, move |_, _, state: &mut Loop| {
        let seen = compositor_model_stats_registry_base::base::VBLANKS.load(Ordering::Relaxed);
        // A paused session produces no flips BY DESIGN (VT switched away, DRM
        // master released). Forcing frames into that is at best wasted and at
        // worst a commit on a device we do not own.
        let paused = matches!(state.inner.status_session, StatusSession::Paused);
        if paused || seen != last_vblanks {
            last_vblanks = seen;
            announced = false;
            return TimeoutAction::ToDuration(IDLE_WINDOW);
        }
        // Once per stall, not once per retry: a desktop that is simply idle —
        // nothing damaged, so the forced frame queues no flip either — would
        // otherwise log every ten seconds forever.
        if !announced {
            warn!(
                "native: no flip in {}s — forcing a redraw (idle rescue)",
                IDLE_WINDOW.as_secs()
            );
            announced = true;
        }
        // Force FULL damage first, and this is what makes the rescue actually
        // rescue. An empty render result queues no page flip at all
        // (`execute.rs`'s `if !last_result_empty`), so a redraw that finds
        // nothing changed produces no vblank — and "nothing changed" is precisely
        // the state a stuck loop is in. Resetting the swapchain ages makes the
        // next frame a full redraw, so a flip is certain.
        //
        // Free at this rate: one full-screen composite per ten seconds, and only
        // while nothing else is drawing at all.
        full_damage();
        // `last_vblanks` is deliberately NOT updated here, so the next tick can
        // tell whether this actually restarted anything.
        state.force_redraw();
        TimeoutAction::ToDuration(IDLE_WINDOW)
    }) {
        Ok(_) => info!("native: idle rescue armed ({}s)", IDLE_WINDOW.as_secs()),
        // Not fatal — everything else still works. But say so: without it the
        // rare freeze needs a VT switch to clear, which is what it exists for.
        Err(e) => warn!("native: idle rescue registration failed: {e}"),
    }
}

//! The post-activation settle watchdog: a bounded, unconditional 30fps kick for
//! the first few seconds after a display comes up.
//!
//! A separate source with a separate condition, and deliberately NO relationship
//! to the exclusive-pacing floor watchdog next door (`watchdog.base`). That one
//! asks "is the pacer still alive" and exists only while a redraw gate is
//! engaged; this one asks nothing and runs whether or not anything is gated.
//! They may overlap freely — both only ever force a redraw, which is idempotent.
//!
//! A SAFEGUARD, not a fix for a diagnosed failure. Unconditional because right
//! after an activation there is nothing reliable to condition on, and bounded
//! because a permanent net would be a second cadence source competing with the
//! real one.
//!
//! The one concrete case behind it is narrow and documented elsewhere: a pipe
//! that has never flipped gets no per-CRTC vblank, so only the redraw ping
//! renders it (`display.reconcile`, `dispatch.state/state.base::force_redraw`).
//! Everything past that is precaution — the resume watchdog beside it retires on
//! the FIRST real vblank, which proves a pipe flipped once rather than that the
//! loop is carrying itself, and this keeps frames coming a few seconds longer.

use compositor_orchestration_core_state_base::Loop;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 30fps: enough to keep the cursor and the compositor's UI live while things
/// settle, cheap enough to be worth wasting for a few seconds.
pub const KICK: Duration = Duration::from_millis(33);

/// How long the net stays up past the most recent activation.
pub const SETTLE: Duration = Duration::from_secs(5);

/// When the current window ends. Re-arming rewrites this rather than registering
/// a second timer: activations arrive in clusters — one hotplug reconcile can
/// light several pipes — and each should EXTEND the net, not multiply the kicks.
static UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

/// Whether a timer source is currently registered against `UNTIL`.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Start, or extend, the settle window. Safe to call from anywhere an activation
/// completes; idempotent within a window.
pub fn arm(loop_handle: &LoopHandle<'static, Loop>) {
    if let Ok(mut until) = UNTIL.lock() {
        *until = Some(Instant::now() + SETTLE);
    }
    // Already running — rewriting the deadline above was the whole update.
    if RUNNING.swap(true, Ordering::Relaxed) {
        return;
    }
    // `TimeoutAction::Drop` retires the source from inside its own callback, so
    // there is no token to hand back, store, or go stale.
    let source = Timer::from_duration(KICK);
    let result = loop_handle.insert_source(source, |_, _, state: &mut Loop| {
        if expired() {
            RUNNING.store(false, Ordering::Relaxed);
            info!("settle: post-activation watchdog finished");
            return TimeoutAction::Drop;
        }
        state.force_redraw();
        TimeoutAction::ToDuration(KICK)
    });
    match result {
        Ok(_) => info!("settle: post-activation watchdog armed for {:?}", SETTLE),
        // Not fatal: without it a pipe that has never flipped can sit dark until
        // some unrelated source forces a full render.
        Err(e) => {
            RUNNING.store(false, Ordering::Relaxed);
            warn!("settle: post-activation watchdog registration failed: {e}");
        }
    }
}

/// Past the deadline — or unable to read it, which retires rather than pins the
/// timer on: a stuck lock should not leave a 30fps kick running forever.
fn expired() -> bool {
    UNTIL
        .lock()
        .map(|until| until.is_none_or(|t| Instant::now() >= t))
        .unwrap_or(true)
}

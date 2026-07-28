//! The user's exclusivity floor, projected into the runtime.
//!
//! `Config::floor` is a `Rate`, and resolving a `Rate::Multiplier` needs the
//! output's refresh interval — which this layer, by design, knows nothing about.
//! So the render loop resolves it against the mode it is driving and publishes
//! the answer here every frame, and the watchdog reads it.
//!
//! Separate from `tearing.liveness` because it is a different thing: this is a
//! setting flowing DOWN, that is measurement flowing UP.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Nanoseconds. Always a real interval — there is no "off" (see
/// `environment.tearing/tearing.config::FLOOR_MIN_FPS`): with a gate engaged the
/// rescue frames are the only thing still carrying the cursor, the compositor's
/// UI and the frame callbacks the admitted client needs before it may commit.
///
/// Seeded at 33ms rather than at `FLOOR_DEFAULT`, which is a multiple of refresh
/// and so cannot be resolved without a mode. This value only governs the window
/// between the watchdog arming and the first frame publishing a real one, and
/// erring slow there is right: it cannot rescue a loop that has not run yet.
static FLOOR_NS: AtomicU64 = AtomicU64::new(33_000_000);

/// The longest the watchdog will EVER wait, whatever a measured cadence says.
/// A 4 FPS target would otherwise push the threshold past a quarter second and
/// take the cursor and the UI down with it.
pub const CEILING: Duration = Duration::from_millis(250);

pub fn set(floor: Duration) {
    FLOOR_NS.store(floor.as_nanos() as u64, Ordering::Relaxed);
}

pub fn get() -> Duration {
    Duration::from_nanos(FLOOR_NS.load(Ordering::Relaxed))
}

/// How often the watchdog should ask — the floor itself, so a fast floor is not
/// answered by a slow timer.
pub fn poll() -> Duration {
    get()
}

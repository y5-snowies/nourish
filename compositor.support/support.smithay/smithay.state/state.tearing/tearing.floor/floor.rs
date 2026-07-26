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

/// Nanoseconds; `0` = no floor, watchdog disabled.
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

pub fn set(floor: Option<Duration>) {
    FLOOR_NS.store(floor.map_or(0, |d| d.as_nanos() as u64), Ordering::Relaxed);
}

/// `None` when the user turned the watchdog off.
pub fn get() -> Option<Duration> {
    match FLOOR_NS.load(Ordering::Relaxed) {
        0 => None,
        ns => Some(Duration::from_nanos(ns)),
    }
}

/// How often the watchdog should ask. Tracks the floor, so a fast floor is not
/// answered by a slow timer; the fallback applies only while disabled, when the
/// answer does not matter.
pub fn poll() -> Duration {
    get().unwrap_or(Duration::from_millis(33))
}

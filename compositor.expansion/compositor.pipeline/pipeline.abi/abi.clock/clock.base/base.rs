//! The one clock a shader's `t` and its window timestamps are both measured on.
//!
//! # Why this exists
//!
//! A window timestamp is only useful next to `t`: what a pass wants is an AGE,
//! `t - times.life[i].x`, and a subtraction between two different clocks is a
//! number with no meaning. Publishing pre-computed ages instead would avoid the
//! question, but an age is only correct for the instant it was computed, and the
//! offloaded path renders on the worker's schedule rather than the compositor's —
//! so an animation driven by one would advance in steps the size of a publish, not
//! of a frame.
//!
//! So the timestamps are ABSOLUTE and the shader does the subtraction, which makes
//! it exact on both paths and at any frame rate.
//!
//! # What it replaced
//!
//! `t` had two origins: `ParallaxBackground::start_time`, stamped per element
//! instance, and the worker's own `start`. A bundle therefore saw a different
//! clock depending on where it happened to be placed, and one instance per pane
//! meant a multi-pane desktop was already animating out of phase with itself. One
//! process-wide origin fixes that as a side effect; the reason it had to change at
//! all is that a timestamp cannot be published against a clock that does not
//! exist yet at publish time.
//!
//! # Precision
//!
//! `f32` seconds since process start. The gap between representable values grows
//! with uptime — about 8 ms after a day — and that is a property of `t` itself,
//! not something introduced here: an age is the difference of two values on this
//! scale, so it inherits exactly the resolution the animation clock already had.

use std::sync::LazyLock;
use std::time::Instant;

/// Process start, as close to it as the first caller.
static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Seconds since [`EPOCH`] — the value a shader receives as `t`, and the scale
/// every published window timestamp is on.
pub fn now() -> f32 {
    EPOCH.elapsed().as_secs_f32()
}

/// A moment that has not happened.
///
/// Negative, so `t - NEVER` is a large positive age rather than a small one: a
/// pass that forgets to test it sees an event long past — an animation already
/// finished — instead of one that just fired. The failure mode of the check being
/// omitted should be "nothing happens", not "everything animates at once".
pub const NEVER: f32 = -1.0;

/// Whether `at` names a moment that actually happened.
pub fn happened(at: f32) -> bool {
    at >= 0.0
}

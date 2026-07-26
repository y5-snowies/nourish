//! Composite liveness per output — what the exclusive-pacing watchdog reads.
//!
//! Split out of `tearing.pacer` because it is a different kind of state: the
//! pacer tag is per-surface protocol data written by a client, this is per-output
//! timing the render loop writes and a timer reads.
//!
//! Per PIPE rather than global. With one output compositing at 300fps and
//! another showing a paced desktop, a single shared "last composite" lets the
//! fast one answer for the slow one, and the slow one is never rescued.

use compositor_support_smithay_state_tearing_floor::floor::{self, CEILING};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One output's composite cadence.
struct Pipe {
    last: Instant,
    /// Smoothed interval between composites. Under exclusivity the pacer is the
    /// only thing producing them, so this IS the pacer's frame rate.
    interval: Option<Duration>,
}

impl Pipe {
    /// Twice the pacer's own cadence, bounded below by the user's floor.
    ///
    /// A target running slower than the floor is slow, not stalled. Firing at the
    /// floor against it interleaves a watchdog frame with nearly every client
    /// frame, which hands back exactly the exclusivity the gate exists to
    /// enforce — precisely when the target can least afford the competition.
    fn threshold(&self, floor: Duration) -> Duration {
        self.interval
            .map_or(floor, |i| (i * 2).clamp(floor, CEILING))
    }
}

static PIPES: Mutex<Vec<(String, Pipe)>> = Mutex::new(Vec::new());

/// Record that `key` composited. Called on every frame, paced or not.
pub fn note_composite(key: &str) {
    let now = Instant::now();
    let Ok(mut pipes) = PIPES.lock() else { return };
    match pipes.iter_mut().find(|(k, _)| k == key) {
        Some((_, p)) => {
            let delta = now.duration_since(p.last);
            // EWMA over ~4 frames: one long composite (a shader hitch, a mode
            // set) must not push the threshold out for the frames behind it.
            p.interval = Some(p.interval.map_or(delta, |prev| (prev * 3 + delta) / 4));
            p.last = now;
        }
        None => pipes.push((key.to_string(), Pipe { last: now, interval: None })),
    }
}

/// Has any pipe gone longer than its OWN threshold without compositing?
///
/// Empty (nothing recorded yet, or just reset) counts as stalled: engagement has
/// just begun and one frame is what establishes the cadence. Always `false` with
/// the floor turned off — the user asked for no rescue.
pub fn stalled() -> bool {
    let Some(floor) = floor::get() else { return false };
    let now = Instant::now();
    PIPES
        .lock()
        .map(|pipes| {
            pipes.is_empty()
                || pipes
                    .iter()
                    .any(|(_, p)| now.duration_since(p.last) >= p.threshold(floor))
        })
        .unwrap_or(true)
}

/// Forget every measured cadence. Called when the gate disengages, so the next
/// engagement measures its own pacer instead of inheriting the previous one's.
pub fn reset() {
    if let Ok(mut pipes) = PIPES.lock() {
        pipes.clear();
    }
}

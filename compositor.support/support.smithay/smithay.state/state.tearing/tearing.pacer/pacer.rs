//! Exclusive-pacing state for `TearingMode::Exclusive`.
//!
//! In that mode only TAGGED clients may drive the flip cadence: their commits
//! schedule redraws, everyone else's do not. Every flip then carries exactly one
//! new frame from a tagged client, so seams-per-second equals their frame rate —
//! the floor for a tearing setup — and a neighbour's damage can no longer spend a
//! flip (and a tear) without advancing anything the user is watching.
//!
//! Deliberately dependency-free: the surface marker is a bare unit type so both
//! the Wayland dispatch (which owns `commit`) and the y5 window layer (which owns
//! introspection) can put it in smithay's `UserDataMap` without this crate
//! needing smithay at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Per-surface "target" tag, in the surface's `UserDataMap`. Written solely by
/// `wp_tearing_control_v1` — a client declaring it wants tearing.
///
/// Toggleable rather than a bare marker because the hint is revocable (a client
/// may go back to `vsync`) and `UserDataMap` entries cannot be removed.
///
/// Read on EVERY commit, so whatever writes it must be cheap. That rules out
/// process introspection, which is why the earlier exec-path heuristic had to
/// resolve once at map time; the protocol has no such cost and replaced it.
#[derive(Debug)]
pub struct PacerSurface(AtomicBool);

impl PacerSurface {
    pub fn new(on: bool) -> Self {
        Self(AtomicBool::new(on))
    }
    pub fn set(&self, on: bool) {
        self.0.store(on, Ordering::Relaxed);
    }
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}



static LAST_COMPOSITE: Mutex<Option<Instant>> = Mutex::new(None);

/// Record that a frame was queued — the watchdog's liveness reference.
pub fn note_composite() {
    if let Ok(mut l) = LAST_COMPOSITE.lock() {
        *l = Some(Instant::now());
    }
}

/// Floor pacing: the longest the compositor may go without producing a frame
/// while a pacer is driving. A tagged client that stalls (loading screen, shader
/// hitch, alt-tab) must not be able to freeze the desktop with it.
pub const FLOOR: Duration = Duration::from_millis(33);

/// Nothing has composited within `FLOOR` — the watchdog should force a frame.
pub fn stalled() -> bool {
    LAST_COMPOSITE
        .lock()
        .map(|l| l.is_none_or(|t| t.elapsed() >= FLOOR))
        .unwrap_or(true)
}

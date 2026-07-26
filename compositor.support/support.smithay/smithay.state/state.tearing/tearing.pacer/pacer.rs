//! The per-surface "target" tag for `TearingMode::Exclusive`. The watchdog's
//! composite liveness lives next door in `tearing.liveness`.
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


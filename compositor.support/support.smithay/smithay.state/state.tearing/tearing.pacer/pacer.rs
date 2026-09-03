//! The per-surface tag deciding whether a window owns the redraw cadence — read by
//! BOTH policy sections. The watchdog's composite liveness lives next door in
//! `tearing.liveness`.
//!
//! Where a section is exclusive only TAGGED clients may drive the flip cadence: their
//! commits schedule redraws, everyone else's do not. Every flip then carries exactly
//! one new frame from a tagged client, so seams-per-second equals their frame rate —
//! the floor for a tearing setup — and a neighbour's damage can no longer spend a flip
//! (and a tear) without advancing anything the user is watching.
//!
//! Deliberately dependency-free: plain `std`, so both the Wayland dispatch (which owns
//! `commit`) and the y5 window layer (which owns introspection) can put it in smithay's
//! `UserDataMap` without this crate needing smithay at all.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const SILENT: u8 = 0;
const TEAR: u8 = 1;
const VSYNC: u8 = 2;

/// The three independent answers to "should this surface's window own the cadence".
///
/// ONE entry rather than three, because the answer is a function of all of them and
/// reading a subset is the bug this replaces twice over: the heuristic first SHARED the
/// client's flag and was erased by every `set_presentation_hint`, and the two call sites
/// that later read them apart drifted into different rules. Each setter still has
/// exactly one caller, so no writer can clobber another's answer.
#[derive(Debug, Default)]
pub struct TearingTag {
    /// The USER, via `Y5_TEARING=1` on the process or an ancestor. Not a guess.
    forced: AtomicBool,
    /// The COMPOSITOR's guess (Steam attribution), from `y5.graphic/graphic.tearing`.
    /// Resolved once at map — it walks `/proc`, far too heavy for the commit path that
    /// reads this.
    heuristic: AtomicBool,
    /// The CLIENT's statement via `wp_tearing_control_v1`. Tri-state: silence differs
    /// from an explicit `vsync`, and the hint is revocable — neither of which
    /// `UserDataMap` (insert-once, never removed) can express through presence.
    hint: AtomicU8,
}

impl TearingTag {
    pub fn set_forced(&self, on: bool) {
        self.forced.store(on, Ordering::Relaxed);
    }
    pub fn forced(&self) -> bool {
        self.forced.load(Ordering::Relaxed)
    }
    pub fn set_heuristic(&self, on: bool) {
        self.heuristic.store(on, Ordering::Relaxed);
    }
    pub fn heuristic(&self) -> bool {
        self.heuristic.load(Ordering::Relaxed)
    }
    pub fn set_hint(&self, tear: bool) {
        self.hint.store(if tear { TEAR } else { VSYNC }, Ordering::Relaxed);
    }
    /// `None` = the client never bound the protocol.
    pub fn hint(&self) -> Option<bool> {
        match self.hint.load(Ordering::Relaxed) {
            SILENT => None,
            v => Some(v == TEAR),
        }
    }
}

/// A whole window's tags, accumulated over its surface TREE — which is where the answer
/// lives: Mesa attaches `wp_tearing_control` to the surface it presents to, for many
/// native games a subsurface under the toplevel, while the heuristic stamps the toplevel.
/// So the halves are gathered separately and resolved once, at the window; resolving per
/// surface would let a toplevel's heuristic outvote the hint its own subsurface gave.
#[derive(Debug, Default, Clone, Copy)]
pub struct Verdict {
    pub forced: bool,
    pub heuristic: bool,
    pub hint: Option<bool>,
}

impl Verdict {
    pub fn absorb(&mut self, tag: &TearingTag) {
        self.forced |= tag.forced();
        self.heuristic |= tag.heuristic();
        if let Some(tear) = tag.hint() {
            self.hint = Some(self.hint.unwrap_or(false) || tear);
        }
    }

    /// The single definition of "is this a target", and the whole precedence ladder:
    /// `Y5_TEARING` (the user, about this exact process) beats `wp_tearing_control_v1`
    /// (the client, about its own flips) beats the Steam guess, which answers only the
    /// silence. Above all three sits a configured `Selector::Always`, which never
    /// consults this at all.
    pub fn is_target(self) -> bool {
        self.forced || self.hint.unwrap_or(self.heuristic)
    }
}

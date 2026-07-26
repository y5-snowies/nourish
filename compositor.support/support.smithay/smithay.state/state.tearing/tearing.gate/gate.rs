//! The commit-time redraw gate: which surfaces may drive the frame cadence.
//!
//! `Exclusivity` (a user setting) resolves per frame into a `Gate` (what the
//! Wayland dispatch enforces per commit). They are separate types on purpose:
//! the dispatch layer sits below orchestration and cannot see policy, scenes or
//! windows — only the surface in front of it and these globals.
//!
//! Hence the home: this is the render loop's channel TO dispatch, written once a
//! frame by the compositor and read on every commit. It is runtime state, not a
//! setting, so it belongs beside the other smithay state and not with the
//! persisted policy in `environment.tearing`.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

/// Which surfaces are allowed to schedule a redraw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gate {
    /// No restriction — every client and every internal source drives.
    #[default]
    Off,
    Tagged,
    TaggedFocused,
    Focused,
    /// Anything the scene actually drew last frame. Wider than `Tagged`, but
    /// still silences offscreen clients and the compositor's own animation
    /// sources, which is the whole point of a gate.
    Visible,
}

impl Gate {
    fn code(self) -> u8 {
        match self {
            Self::Off => 0, Self::Tagged => 1, Self::TaggedFocused => 2,
            Self::Focused => 3, Self::Visible => 4,
        }
    }
    fn from_code(c: u8) -> Self {
        match c {
            1 => Self::Tagged, 2 => Self::TaggedFocused,
            3 => Self::Focused, 4 => Self::Visible, _ => Self::Off,
        }
    }

    /// Does a surface with these properties pass? `Off` is handled by the caller
    /// (it takes the unrestricted path), so it is permissive here.
    ///
    /// The tagged gates require `visible` too, and that is what scopes them to
    /// the world you are actually looking at. The dispatch layer cannot know what
    /// a world is, but the scene stamps only what it drew, and it only ever draws
    /// the active one — so a tagged window parked in a background world stops
    /// driving the cadence without anyone here having to reason about worlds.
    pub fn admits(self, tagged: bool, focused: bool, visible: bool) -> bool {
        match self {
            Self::Off => true,
            Self::Tagged => tagged && visible,
            Self::TaggedFocused => tagged && focused && visible,
            Self::Focused => focused,
            Self::Visible => visible,
        }
    }
}

static GATE: AtomicU8 = AtomicU8::new(0);

/// Returns `true` when this call CHANGED the gate.
pub fn set(g: Gate) -> bool { GATE.swap(g.code(), Ordering::Relaxed) != g.code() }
pub fn get() -> Gate { Gate::from_code(GATE.load(Ordering::Relaxed)) }
pub fn engaged() -> bool { get() != Gate::Off }

/// Frame index the scene last drew a surface, in its `UserDataMap`. A stamp
/// rather than a flag because there is no moment at which every surface can be
/// cleared — the scene knows what it drew, never what it didn't.
#[derive(Debug, Default)]
pub struct VisibleSurface(AtomicU64);

impl VisibleSurface {
    pub fn stamp(&self, frame: u64) { self.0.store(frame, Ordering::Relaxed); }
    /// Drawn in the last two frames — slack for a commit arriving between the
    /// scene build and the next one.
    pub fn fresh(&self, frame: u64) -> bool {
        frame.saturating_sub(self.0.load(Ordering::Relaxed)) <= 1
    }
}

static FRAME: AtomicU64 = AtomicU64::new(0);

pub fn frame() -> u64 { FRAME.load(Ordering::Relaxed) }
pub fn advance_frame() -> u64 { FRAME.fetch_add(1, Ordering::Relaxed) + 1 }

/// Whether the resolved policy could tear, published for the NEXT frame's plane
/// assignment (which is decided before that frame's scene exists).
static TEARING: AtomicBool = AtomicBool::new(false);

pub fn set_tearing(on: bool) -> bool { TEARING.swap(on, Ordering::Relaxed) != on }
pub fn tearing() -> bool { TEARING.load(Ordering::Relaxed) }

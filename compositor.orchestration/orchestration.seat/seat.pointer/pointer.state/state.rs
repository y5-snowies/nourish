use std::sync::Arc;

use smithay::input::pointer::CursorIcon;
use smithay::utils::{Logical, Point};
use compositor_orchestration_seat_pointer_element::element::PointerElement;
use compositor_orchestration_seat_pointer_texture::pointer_load::CursorThemeCache;

pub struct PointerState {
    pub motion: Point<f64, Logical>,
    pub element: PointerElement,
    /// Edge-pan autoscroll armed by an ABSOLUTE pointer (winit) sitting inside the
    /// edge band. A relative pointer's overflow past the extent IS its pan step, but
    /// an absolute one is clamped to the output and stops reporting once it is
    /// pressed against the edge, so the pan has to run off the frame clock instead.
    /// `None` = not in the band. Set/cleared by the seat's `pointer.input/extent`,
    /// stepped by the per-frame hook.
    pub edge_hold: Option<EdgeHold>,
}

/// An absolute pointer parked in the edge band: the canvas travel it is asking for,
/// and the position to replay each frame so the world point under it keeps up with
/// the camera.
#[derive(Clone, Copy)]
pub struct EdgeHold {
    /// Which extent is held, per axis: `-1` low edge, `+1` high edge, `0` none.
    pub dir_x: f64,
    pub dir_y: f64,
    /// Camera travel in physical px/sec, signed per axis (right/down positive).
    /// Zero while [`seed`](Self::seed) is still measuring.
    pub vx: f64,
    pub vy: f64,
    /// Arrival sampling, still open. `None` once the sustained speed is fixed (and
    /// from the outset for an absolute hold, whose speed comes from the band depth).
    pub seed: Option<EdgeSeed>,
    /// Armed by an ABSOLUTE pointer sitting in the edge band, rather than by a
    /// relative one parked against the extent. Only the absolute kind ends on button
    /// release: what was feeding it is the host's implicit drag grab. A relative
    /// continuous pan outlives the button, because the cursor is still at the edge.
    pub absolute: bool,
    /// The reported position, normalized to the output (may sit outside `0..=1`
    /// while a winit drag holds the host's implicit grab), replayed per frame.
    pub nx: f64,
    pub ny: f64,
    /// When the last autoscroll step ran.
    pub last: std::time::Instant,
}

/// The arrival measurement behind a continuous edge pan: how hard the pointer drove
/// into the extent over the first moments of being parked there. Slam into the edge
/// and the canvas keeps moving fast; creep into it and it crawls.
///
/// Speed is `push / active` — distance driven into the edge over the time spent
/// driving it, NOT over the window's wall clock. Those differ whenever the user
/// shoves and stops, which is the common case: dividing a hard 30ms shove by a 120ms
/// window would report a quarter of the speed actually travelled, and the pan would
/// visibly sag the moment the hand stopped.
#[derive(Clone, Copy)]
pub struct EdgeSeed {
    /// When the pointer parked (the window opened).
    pub since: std::time::Instant,
    /// Push distance accumulated into the edge so far, physical px, per axis.
    pub push_x: f64,
    pub push_y: f64,
    /// Seconds over which that distance arrived, per axis. Only intervals that
    /// actually carried a push count, so pauses never dilute the measurement.
    pub active_x: f64,
    pub active_y: f64,
}

impl PointerState {
    pub fn new() -> PointerState {
        let theme_name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "Adwaita".into());
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);

            
        let theme = Arc::new(CursorThemeCache::new(&theme_name, size));

        let pointer_element = PointerElement::new(theme);

        return PointerState {
            motion: Point::new(0.0, 0.0),
            element: pointer_element,
            edge_hold: None,
        };
    }
}

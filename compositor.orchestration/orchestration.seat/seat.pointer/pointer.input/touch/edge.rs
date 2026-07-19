//! Edge-swipe recognizer. A 2-finger swipe that BEGINS at a screen edge is
//! swallowed by the compositor — it never reaches the camera or a client — and
//! dispatched as a directional `(edge, angle)` event. The recognizer is general;
//! today the only wired binding is the LEFT edge near vertical centre swiped
//! inward → toggle the sticky touch pane.
use smithay::utils::{Physical, Point};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_seat_gesture_touch::touch::{Role, TouchEdge};

/// Distance from a screen edge (physical px) within which the 2-finger centroid
/// counts as "at the utmost edge".
const EDGE_BAND: f64 = 40.0;
/// Minimum centroid travel (physical px) before an edge swipe fires.
const FIRE_DISTANCE: f64 = 60.0;

/// Which edge (if any) a physical screen point sits against.
pub fn classify(_loop: &Loop, p: Point<f64, Physical>) -> Option<TouchEdge> {
    let (w, h) = _loop.size_ctx_all().screen_size_physical;
    if p.x <= EDGE_BAND {
        Some(TouchEdge::Left)
    } else if p.x >= w - EDGE_BAND {
        Some(TouchEdge::Right)
    } else if p.y <= EDGE_BAND {
        Some(TouchEdge::Top)
    } else if p.y >= h - EDGE_BAND {
        Some(TouchEdge::Bottom)
    } else {
        None
    }
}

/// Called at the 2-finger transition: if the contact centroid is at a screen
/// edge, claim the sequence as an edge swipe (`Role::Edge`) and latch its origin.
/// Returns whether it claimed the sequence (so the caller skips the camera path).
pub fn maybe_begin(_loop: &mut Loop, _time: u32) -> bool {
    let c = _loop.inner.touch.centroid();
    let Some(edge) = classify(_loop, c) else {
        return false;
    };
    _loop.inner.touch.role = Role::Edge;
    _loop.inner.touch.start_edge = Some(edge);
    _loop.inner.touch.edge_start = c;
    _loop.inner.touch.edge_fired = false;
    true
}

/// Per-motion update for `Role::Edge`: once the centroid has travelled far enough,
/// fire the binding exactly once with the swipe angle.
pub fn update(_loop: &mut Loop, _time: u32) {
    if _loop.inner.touch.edge_fired {
        return;
    }
    let c = _loop.inner.touch.centroid();
    let start = _loop.inner.touch.edge_start;
    let (dx, dy) = (c.x - start.x, c.y - start.y);
    if dx.hypot(dy) < FIRE_DISTANCE {
        return;
    }
    let Some(edge) = _loop.inner.touch.start_edge else {
        return;
    };
    let angle = dy.atan2(dx).to_degrees();
    _loop.inner.touch.edge_fired = true;
    dispatch(_loop, edge, start, angle);
}

/// Map a fired edge swipe to its action. `angle` degrees: 0 = right, 90 = down.
/// Only the left-edge / vertical-centre / inward swipe is wired now (toggle the
/// touch pane); other edges are recognized but unbound.
fn dispatch(_loop: &mut Loop, edge: TouchEdge, start: Point<f64, Physical>, angle: f64) {
    let (_, h) = _loop.size_ctx_all().screen_size_physical;
    let centred_y = start.y > h * 0.25 && start.y < h * 0.75;
    if edge == TouchEdge::Left && centred_y && angle.abs() < 60.0 {
        // Toggle the pane, pinned to the world it was summoned in.
        let here = _loop.inner.worlds.spawn_target().as_u128();
        _loop.inner.touch.pane_world = match _loop.inner.touch.pane_world {
            Some(w) if w == here => None,
            _ => Some(here),
        };
    }
}

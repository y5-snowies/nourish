use smithay::{
    input::pointer::PointerHandle,
    utils::{Logical, Point},
    wayland::{
        pointer_constraints::{PointerConstraint, PointerConstraintRef, with_pointer_constraint},
        seat::WaylandFocus,
    },
};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_camera_transform_translate::map;
use compositor_y5_canvas_input_state::state::{ActiveOption, CanvasGrab};

/// The hand tool owns the pointer while it is active: no client lock or confine
/// applies to it, none may activate, and the client is not fed relative motion
/// (`native_motion::dispatch`) — the pan is the canvas's, not the game's.
pub(crate) fn hand_active(_loop: &Loop) -> bool {
    matches!(_loop.inner.canvas().Grab, CanvasGrab::Active(ActiveOption::Hand))
}

/// Break the focused client's pointer lock/confine outright and suspend
/// activation — the canvas is taking the pointer (hand tool engaged). Called at
/// the hand tool's entry points; [`sync`] catches every exit.
pub fn break_constraint(_loop: &mut Loop) {
    if let Some(pointer) = _loop.state.seat.seat.get_pointer() {
        _loop.state.suspend_constraints(&pointer);
    }
}

/// Keep the seat's suspension in step with the hand tool, on every motion:
/// the tool has several exits (its chord, Escape, the tablet toggle, ...), and
/// this is the one place that sees all of them. Returns whether the hand is on.
fn sync(_loop: &mut Loop) -> bool {
    let hand = hand_active(_loop);
    if hand != _loop.state.seat.constraints_suspended {
        if let Some(pointer) = _loop.state.seat.seat.get_pointer() {
            match hand {
                true => _loop.state.suspend_constraints(&pointer),
                false => _loop.state.resume_constraints(&pointer),
            }
        }
    }
    hand
}

pub fn apply_pointer_constraint(
    _loop: &mut Loop,
    previous_pointer_location: Point<f64, Logical>,
    candidate: Point<f64, Logical>,
) -> (Point<f64, Logical>, bool) {
    if sync(_loop) {
        return (candidate, false);
    }
    let Some(pointer) = _loop.state.seat.seat.get_pointer() else {
        return (candidate, false);
    };

    let Some(focused) = pointer.current_focus() else {
        return (candidate, false);
    };
    let Some(focused_surface) = focused.wl_surface() else {
        return (candidate, false);
    };

    // Resolved BEFORE `with_pointer_constraint` takes the surface's per-surface
    // data lock: the mapping reads surface state itself, and re-entering that
    // non-reentrant mutex from inside the closure deadlocks the main thread.
    //
    // The map is the fit the renderer applies, resolved for THIS surface (the
    // focused one may be a subsurface — a game's presentation surface), so the
    // confine test happens in the client's own surface-local space against what
    // is actually on screen.
    let map = map::surface_map(&_loop.inner.space_state().state, &focused_surface);

    let mut result = candidate;
    let mut active = false;

    with_pointer_constraint(&focused_surface, &pointer, |constraint| {
        let Some(constraint) = constraint else { return };
        if !constraint.is_active() { return }
        active = true;

        match &*constraint {
            PointerConstraint::Locked(_) => {
                result = previous_pointer_location;
            }
            PointerConstraint::Confined(confined) => {
                let Some(map) = map.as_ref() else {
                    // No window owns the surface: nothing to confine to but where we are.
                    result = previous_pointer_location;
                    return;
                };
                let local = map.to_local(candidate);
                let inside = match confined.region() {
                    Some(region) => region.contains((local.x.floor() as i32, local.y.floor() as i32)),
                    // No region: the whole surface, at its real (viewport / scaled) extent.
                    None => {
                        local.x >= 0.0
                            && local.y >= 0.0
                            && local.x < map.extent.w as f64
                            && local.y < map.extent.h as f64
                    }
                };
                result = if inside { candidate } else { previous_pointer_location };
            }
        }
    });

    (result, active)
}

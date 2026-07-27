//! Pointer button for the embedded globe (overview World tab): route to the
//! picker's own button handler (drag rotates, click focuses a cell) and report
//! whether this was a CLICK on the already-focused cell — the caller then enters
//! that world ("click a selected cell again to enter", instead of pressing
//! Enter).

use smithay::backend::input::{ButtonState, InputBackend, PointerButtonEvent};
use smithay::utils::{Logical, Physical, Point};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_picker_state_base::base::PickerActive;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

/// Press→release within this many px counts as a click, not a drag (mirrors the
/// picker's own threshold).
const CLICK_PX: f64 = 6.0;

fn active(state: &mut Loop) -> Option<&mut PickerActive> {
    state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).active.as_mut()
}

/// The seat cursor's position in physical/screen px (the picker's pointer space).
fn seat_pointer_physical(state: &mut Loop) -> Option<(f64, f64)> {
    let loc: Point<f64, Logical> = state.state.seat.seat.get_pointer()?.current_location();
    let ctx = state.size_ctx_all();
    let t: compositor_y5_camera_transform_translate::transform::Transform = (loc, ctx).into();
    let phys: Point<f64, Physical> = t.into();
    Some((phys.x, phys.y))
}

/// Embedded-globe pointer motion: sync the picker pointer FROM the real seat
/// cursor and rotate while dragging. The full-screen picker dead-reckons
/// `active.pointer` from relative deltas (starting at 0,0) and warps the seat
/// cursor to match — but in the embed the seat keeps owning the cursor, so the
/// dead-reckoned position diverges and the warp fights the seat (the globe
/// ignored the mouse until a tab round-trip re-aligned it).
pub fn embed_motion(state: &mut Loop) {
    let Some((x, y)) = seat_pointer_physical(state) else { return };
    let (_, h) = state.size_ctx_all().screen_size_physical;
    let k = compositor_y5_picker_three_constant::ROTATE_SENSITIVITY as f64 / h.max(1.0);
    let inc = active(state).and_then(|a| {
        let prev = a.pointer;
        a.pointer = (x, y);
        a.drag.is_some().then(|| (a.orientation, ((x - prev.0) * k) as f32, ((y - prev.1) * k) as f32))
    });
    if let Some((o, dx, dy)) = inc {
        let (new_o, spin) = compositor_y5_picker_three_orient::orient::drag(o, dx, dy);
        if let Some(a) = active(state) {
            (a.orientation, a.target) = (new_o, new_o); // free-look; no animate-back
            a.spin = spin; // last increment seeds release momentum
        }
    }
}

pub fn embed_button<I: InputBackend>(
    event: &<I as InputBackend>::PointerButtonEvent,
    state: &mut Loop,
) -> bool {
    if event.state() == ButtonState::Pressed {
        // Sync before drag-start: a press with no prior motion (right after the
        // tab opened) must not hit-test against the session's initial (0,0).
        embed_motion(state);
        compositor_y5_picker_seat_pointer::pointer::button::<I>(event, state);
        return false;
    }
    // Release: capture whether it was a click + the focused cell BEFORE the
    // picker re-selects, then route the release (which performs the select).
    let (prev, was_click) = match active(state) {
        Some(a) => {
            let click = a
                .drag
                .map(|(sx, sy)| (a.pointer.0 - sx).hypot(a.pointer.1 - sy) < CLICK_PX)
                .unwrap_or(false);
            (a.selected, click)
        }
        None => (None, false),
    };
    // Hit-test the release EXPLICITLY. Comparing `selected` before/after the
    // routed release is not enough: a click that misses the sphere leaves
    // `selected` untouched, so "unchanged" read as "re-clicked the focused cell"
    // and dropped the user into that world. Entering requires landing ON it.
    let hit = if was_click { picked(state) } else { None };
    compositor_y5_picker_seat_pointer::pointer::button::<I>(event, state);
    hit.is_some() && hit == prev
}

/// The cell under the picker pointer right now — `None` when the ray misses the
/// sphere's silhouette (the pointer is outside the globe).
fn picked(state: &mut Loop) -> Option<usize> {
    let (pointer, orientation) = active(state).map(|a| (a.pointer, a.orientation))?;
    let output = state.size_ctx_all().screen_size_physical;
    compositor_y5_picker_pick_base::base::pick_cell(pointer, output, orientation)
}

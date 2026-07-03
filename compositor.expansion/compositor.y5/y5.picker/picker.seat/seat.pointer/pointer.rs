//! Picker pointer input: drag rotates; click picks a cell (click again = enter); scroll zooms.
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, InputBackend, PointerAxisEvent, PointerButtonEvent,
    PointerMotionEvent,
};
use smithay::utils::{Logical, Point, Size};
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_state_base::base::PickerActive;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};
use compositor_y5_picker_three_constant::{ROTATE_SENSITIVITY, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP};
use compositor_y5_picker_three_orient::orient::IDENTITY;

const BTN_LEFT: u32 = 0x110;
/// A press→release moving less than this (px) is a click, not a drag.
const CLICK_PX: f64 = 6.0;
fn output_size(state: &mut Loop) -> (f64, f64) { state.size_ctx_all().screen_size_physical }

fn active(state: &mut Loop) -> Option<&mut PickerActive> {
    state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).active.as_mut()
}

pub fn button<I: InputBackend>(event: &<I as InputBackend>::PointerButtonEvent, state: &mut Loop) {
    if event.button_code() != BTN_LEFT {
        return;
    }
    let pressed = event.state() == ButtonState::Pressed;
    if compositor_y5_picker_seat_iced::iced::route_button(state, event.button_code(), pressed) {
        return;
    }
    if !pressed {
        let info = active(state).and_then(|a| a.drag.take().map(|s| (s, a.pointer)));
        if let Some((start, pos)) = info
            && (pos.0 - start.0).hypot(pos.1 - start.1) < CLICK_PX
        {
            let (w, h) = output_size(state);
            let q = active(state).map(|a| a.orientation).unwrap_or(IDENTITY);
            if let Some(c) = compositor_y5_picker_pick_base::base::pick_cell(pos, (w, h), q) {
                if active(state).and_then(|a| a.selected) == Some(c) {
                    compositor_y5_picker_world_base::base::start(state);
                } else {
                    compositor_y5_picker_command_base::base::set_selected(state, Some(c));
                }
            }
        }
        return;
    }
    if let Some(a) = active(state) {
        a.drag = Some(a.pointer);
        a.spin = IDENTITY;
    }
}

pub fn axis<I: InputBackend>(event: &<I as InputBackend>::PointerAxisEvent, state: &mut Loop) {
    // Normalise to notches (wheel = v120/120; winit continuous = `amount`/8) + cap per event.
    let delta = event.amount_v120(Axis::Vertical).map(|v| v / 120.0).or_else(|| event.amount(Axis::Vertical).map(|a| a / 8.0)).unwrap_or(0.0) as f32;
    if let Some(a) = active(state) {
        a.zoom = (a.zoom - (delta * ZOOM_STEP).clamp(-0.2, 0.2)).clamp(ZOOM_MIN, ZOOM_MAX);
    }
}

pub fn absolute<I: InputBackend>(
    event: &<I as InputBackend>::PointerMotionAbsoluteEvent,
    state: &mut Loop,
) {
    let (w, h) = output_size(state);
    let size = Size::<i32, Logical>::from((w.round() as i32, h.round() as i32));
    let p: Point<f64, Logical> = event.position_transformed(size);
    motion(state, p.x, p.y);
}

pub fn relative<I: InputBackend>(event: &<I as InputBackend>::PointerMotionEvent, state: &mut Loop) {
    let d = event.delta();
    let pos = active(state).map(|a| a.pointer).unwrap_or((0.0, 0.0));
    motion(state, pos.0 + d.x, pos.1 + d.y);
}

fn motion(state: &mut Loop, x: f64, y: f64) {
    let (_, h) = output_size(state);
    let k = ROTATE_SENSITIVITY as f64 / h.max(1.0);
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
    compositor_y5_picker_seat_cursor::cursor::update(state, x, y); // cursor follows
}

//! Overview pointer input, encapsulated. The seat delegates here first; each fn
//! returns true if the overview consumed the event (windows get nothing; the
//! menu bar + World-tab globe do).
use smithay::backend::input::{Axis, ButtonState, InputBackend, PointerAxisEvent, PointerButtonEvent};
use smithay::utils::{Logical, Physical, Point};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_monitor_compositor_iced_base::IcedSpace;
use compositor_y5_surface_interface_base::hit::surface_under_filtered;

fn over_bar(state: &mut Loop, loc: Point<f64, Logical>) -> bool {
    surface_under_filtered(state, loc, &|h| h.iced_space() == Some(IcedSpace::Screen)).is_some()
}
fn cursor(state: &mut Loop) -> Point<f64, Logical> {
    state.state.seat.seat.get_pointer().unwrap().current_location()
}

/// Menu bar (screen iced) first; else World tab → globe (re-click a cell enters);
/// else Layout tab → click a window cell to close + view it.
pub fn button<I: InputBackend>(event: &<I as InputBackend>::PointerButtonEvent, state: &mut Loop) -> bool {
    if !state.inner.overview().visible {
        return false;
    }
    let pressed = event.state() == ButtonState::Pressed;
    let loc = cursor(state);
    // A press outside any screen surface (menu bar / popup) dismisses an open
    // logout popup instead of falling through to the grid/globe.
    if pressed && state.inner.overview().logout.is_some() && !over_bar(state, loc) {
        compositor_y5_overview_interface_base::base::toggle_logout(state);
        return true;
    }
    let bar = surface_under_filtered(state, loc, &|h| h.iced_space() == Some(IcedSpace::Screen))
        .and_then(|h| h.iced_handle());
    if let Some(handle) = bar {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            if pressed {
                reg.set_keyboard_focus(Some(handle));
            }
            reg.dispatch_button(Some(handle), event.button_code(), pressed);
        }
        return true;
    }
    if state.inner.overview().is_world() {
        if compositor_y5_picker_seat_embed::embed::embed_button::<I>(event, state) {
            compositor_y5_overview_interface_activate::activate::activate_world(state);
        }
        return true;
    }
    if pressed {
        // Cursor is WORLD space; cells are screen/physical — project via camera.
        let ctx = state.size_ctx_all();
        let projected: compositor_y5_camera_transform_translate::transform::Transform = (loc, ctx).into();
        let phys: Point<f64, Physical> = projected.into();
        let p = Point::<i32, Physical>::from((phys.x.round() as i32, phys.y.round() as i32));
        let cell = state.inner.overview().cells.iter().find(|(_, r)| r.contains(p)).map(|(u, _)| *u);
        if let Some(uuid) = cell {
            compositor_y5_overview_interface_activate::activate::activate(state, uuid);
        }
    }
    true
}

pub fn axis<I: InputBackend>(event: &<I as InputBackend>::PointerAxisEvent, state: &mut Loop) -> bool {
    if !state.inner.overview().visible {
        return false;
    }
    if state.inner.overview().is_world() {
        compositor_y5_picker_seat_pointer::pointer::axis::<I>(event, state);
    } else if state.inner.overview().is_settings() {
        // Settings screen has its own scroll lists → forward the wheel to the iced
        // surface (don't swallow it as grid scroll).
        let handle = state.inner.kernel.get(&compositor_orchestration_driver_settings_base::base::SETTINGS).handle;
        if let Some(handle) = handle {
            let dxd = event.amount_v120(Axis::Horizontal).map(|v| (v / 120.0) as i32).unwrap_or(0);
            let dyd = event.amount_v120(Axis::Vertical).map(|v| (v / 120.0) as i32).unwrap_or(0);
            let dx = event.amount(Axis::Horizontal).unwrap_or(0.0);
            let dy = event.amount(Axis::Vertical).unwrap_or(0.0);
            if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                reg.dispatch_axis(Some(handle), dxd, dyd, dx, dy);
            }
        }
    } else {
        let dy = event
            .amount(Axis::Vertical)
            .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.0);
        state.inner.overview_mut().scroll += dy * 4.0;
    }
    true
}

pub fn relative<I: InputBackend>(event: &<I as InputBackend>::PointerMotionEvent, state: &mut Loop) -> bool {
    if !(state.inner.overview().visible && state.inner.overview().is_world()) {
        return false;
    }
    // Drive globe rotation while over the globe, but NEVER consume motion: the seat
    // cursor must keep tracking the pointer. Consuming it froze the cursor over the
    // globe, so it jumped when the pointer reached the header.
    let loc = cursor(state);
    if !over_bar(state, loc) {
        compositor_y5_picker_seat_pointer::pointer::relative::<I>(event, state);
    }
    false
}

pub fn absolute<I: InputBackend>(event: &<I as InputBackend>::PointerMotionAbsoluteEvent, state: &mut Loop) -> bool {
    if !(state.inner.overview().visible && state.inner.overview().is_world()) {
        return false;
    }
    let loc = cursor(state);
    if !over_bar(state, loc) {
        compositor_y5_picker_seat_pointer::pointer::absolute::<I>(event, state);
    }
    false
}

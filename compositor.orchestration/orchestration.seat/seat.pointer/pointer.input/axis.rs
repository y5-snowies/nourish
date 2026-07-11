use crate::native_axis;
use smithay::backend::input::{AxisSource, InputBackend, PointerAxisEvent as _};
use compositor_orchestration_core_state_base::Loop;

pub fn axis<I: InputBackend>(event: &<I as InputBackend>::PointerAxisEvent, _loop: &mut Loop) {
    // Overview overlay open → axis is fully swallowed (before the world input
    // bus, so the hidden world's camera never reacts). World tab: zoom the globe
    // via the picker's handler. Otherwise: scroll the grid (positive vertical =
    // wheel/finger down reveals lower rows; the render path clamps it).
    // Overview open → the overview layer handles the axis (globe zoom / grid
    // scroll) and swallows it.
    if compositor_y5_overview_input_pointer::pointer::axis::<I>(event, _loop) {
        return;
    }
    {
        let location = _loop.state.seat.seat.get_pointer().unwrap().current_location();
        let h = smithay::backend::input::Axis::Horizontal;
        let v = smithay::backend::input::Axis::Vertical;
        // Touchpad two-finger scroll (libinput `Finger` source) pans the canvas;
        // a discrete wheel keeps zooming. ONLY `Finger` counts — `Continuous` is
        // what the winit/nested backend reports for ALL pixel-delta scroll, so
        // including it made winit treat every wheel scroll as a momentum pan.
        let finger = matches!(event.source(), AxisSource::Finger);
        let mut horizontal = event.amount(h).unwrap_or_else(|| event.amount_v120(h).unwrap_or(0.0));
        let mut vertical = event.amount(v).unwrap_or_else(|| event.amount_v120(v).unwrap_or(0.0));
        // Natural scrolling: invert the finger-axis direction for canvas pan
        // (a discrete wheel is left alone). Mirrors the inversion native_axis
        // applies to the window-scroll path, so both agree.
        if finger && _loop.inner.preference.input_natural_scroll {
            horizontal = -horizontal;
            vertical = -vertical;
        }
        let ev = compositor_support_system_input_event_base::base::InputEvent::PointerAxis {
            horizontal,
            vertical,
            x: location.x,
            y: location.y,
            finger,
            // A real touchpad glide always feeds momentum; touch overrides per gesture.
            momentum: true,
        };
        if compositor_orchestration_input_drive_base::drive::route(_loop, ev)
            == compositor_support_system_input_event_base::base::InputFlow::Consume
        {
            return;
        }
    }
    // Pass from the bus means the cursor is over a window and not a hand-pan
    // (CameraSystem::input consumes the canvas-zoom case), so it's a window scroll.
    let pointer = _loop.state.seat.seat.get_pointer().unwrap();
    native_axis::axis::input_received::<I>(pointer, event, _loop);
}

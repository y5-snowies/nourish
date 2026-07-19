use smithay::backend::input::{Axis, AxisSource, Event, InputBackend, PointerAxisEvent};
use smithay::input::pointer::{AxisFrame, PointerHandle};
use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;

/// Overall speed of a two-finger touchpad (libinput `Finger`) scroll forwarded to
/// a window; `<1.0` slows it. Discrete wheels are unaffected. Tuning knob.
const FINGER_SCROLL_SCALE: f64 = 0.6;

pub fn input_received<I: InputBackend>(
    pointer: PointerHandle<Dispatch>,
    event: &I::PointerAxisEvent,
    _loop: &mut Loop,
) {
    // Overview overlay open → axis scrolls the grid and is swallowed (windows and
    // iced get nothing). Positive vertical (wheel/finger down) reveals lower rows;
    // the render path clamps the offset to the content.
    if _loop.inner.overview().visible {
        let dy = event
            .amount(Axis::Vertical)
            .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.0);
        _loop.inner.overview_mut().scroll += dy * 4.0;
        return;
    }

    let source = event.source();

    let horizontal_amount = event
        .amount(Axis::Horizontal)
        .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
    let vertical_amount = event
        .amount(Axis::Vertical)
        .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
    let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
    let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

    // Natural scrolling: invert the finger-axis direction sent to the window
    // (and iced), matching the inversion the canvas-pan path applies. A discrete
    // wheel is left untouched. `stop`/zero detection below reads the raw event,
    // so flipping the forwarded amounts here does not affect it.
    let invert = matches!(source, AxisSource::Finger) && _loop.inner.preference.input_natural_scroll;
    let sign = if invert { -1.0 } else { 1.0 };
    let mut horizontal_amount = horizontal_amount * sign;
    let mut vertical_amount = vertical_amount * sign;
    let horizontal_amount_discrete = horizontal_amount_discrete.map(|d| d * sign);
    let vertical_amount_discrete = vertical_amount_discrete.map(|d| d * sign);

    // Two-finger scroll to a window: libinput dumps the accumulated pre-recognition
    // distance in the first event, so the gesture starts with a lurch. Ramp the
    // finger-source amount up over the first few events; the stop event resets it.
    // (Discrete/v120 wheels have no such start burst and are left untouched.)
    if source == AxisSource::Finger {
        if event.amount(Axis::Horizontal) == Some(0.0) && event.amount(Axis::Vertical) == Some(0.0) {
            _loop.inner.finger_scroll_ramp.end();
        } else {
            let f = FINGER_SCROLL_SCALE * _loop.inner.finger_scroll_ramp.factor(event.time_msec());
            horizontal_amount *= f;
            vertical_amount *= f;
        }
    }

    let mut frame = AxisFrame::new(event.time_msec()).source(source);
    if horizontal_amount != 0.0 {
        frame = frame.value(Axis::Horizontal, horizontal_amount);
        if let Some(discrete) = horizontal_amount_discrete {
            frame = frame.v120(Axis::Horizontal, discrete as i32);
        }
    }
    if vertical_amount != 0.0 {
        frame = frame.value(Axis::Vertical, vertical_amount);
        if let Some(discrete) = vertical_amount_discrete {
            frame = frame.v120(Axis::Vertical, discrete as i32);
        }
    }

    if source == AxisSource::Finger {
        if event.amount(Axis::Horizontal) == Some(0.0) {
            frame = frame.stop(Axis::Horizontal);
        }
        if event.amount(Axis::Vertical) == Some(0.0) {
            frame = frame.stop(Axis::Vertical);
        }
    }

    pointer.axis(&mut _loop.state, frame);
    pointer.frame(&mut _loop.state);

    if let Some(registry) = _loop.inner.surface_mut().registry.as_mut() {
        // ── Iced axis ─────────────────────────────────────────────────
        let iced_target = registry.pointer_target();
        let discrete_x = horizontal_amount_discrete
            .map(|v| (v / 120.0) as i32)
            .unwrap_or(0);
        let discrete_y = vertical_amount_discrete
            .map(|v| (v / 120.0) as i32)
            .unwrap_or(0);
        registry.dispatch_axis(
            iced_target,
            discrete_x,
            discrete_y,
            horizontal_amount,
            vertical_amount,
        );
    }
}

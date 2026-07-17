//! Pointer-emulation primitives. A single finger on the desktop / compositor UI
//! drives the pointer through the very same entry points a mouse uses (so canvas
//! grab, window focus, viewport and overview all behave identically). The session
//! decides *when* to call these; here we only synthesize the pointer events.
use super::backend::{AbsEvent, BTN_LEFT, BTN_RIGHT, BtnEvent, TouchEmu};
use smithay::backend::input::{AbsolutePositionEvent, ButtonState, InputBackend};

/// Reference width used to read a touch event's normalized 0..1 position via the
/// backend-agnostic `x_transformed(width)` (both real backends compute it as
/// `normalized * width`, so dividing back out recovers the fraction).
const REF: i32 = 1_000_000;

/// Base gain applied to touch pan deltas (both the 2-finger strict pan and the
/// single-finger glide). Finger deltas arrive in *physical* pixels, so on a
/// high-DPI touchscreen a raw 1:1 pan feels twice as fast as the finger moves;
/// this damps it to a comfortable default. It is multiplied by the user's
/// `input_touch_pan_speed` preference (default `1.0`). Touch-only — the
/// trackpad/mouse axis path never routes through here.
const PAN_SPEED: f64 = 0.5;

/// Normalized 0..1 position of a touch (down/motion) event.
pub fn fraction<I: InputBackend, E: AbsolutePositionEvent<I>>(event: &E) -> (f64, f64) {
    (
        event.x_transformed(REF) / REF as f64,
        event.y_transformed(REF) / REF as f64,
    )
}

/// Warp the pointer to a normalized touch point (full motion routing).
pub fn move_to(_loop: &mut compositor_orchestration_core_state_base::Loop, nx: f64, ny: f64, time: u32) {
    crate::motion::absolute::<TouchEmu>(&AbsEvent { time, nx, ny }, _loop);
}

/// Emulate a left-button press at the current pointer location.
pub fn press(_loop: &mut compositor_orchestration_core_state_base::Loop, time: u32) {
    crate::button::button::<TouchEmu>(
        &BtnEvent { time, state: ButtonState::Pressed, button: BTN_LEFT },
        _loop,
    );
}

/// Emulate a left-button release.
pub fn release(_loop: &mut compositor_orchestration_core_state_base::Loop, time: u32) {
    crate::button::button::<TouchEmu>(
        &BtnEvent { time, state: ButtonState::Released, button: BTN_LEFT },
        _loop,
    );
}

/// Emulate a full right-button click (press + release) at the current pointer
/// location — the pointer-mode 2-finger tap.
pub fn right_click(_loop: &mut compositor_orchestration_core_state_base::Loop, time: u32) {
    crate::button::button::<TouchEmu>(
        &BtnEvent { time, state: ButtonState::Pressed, button: BTN_RIGHT },
        _loop,
    );
    crate::button::button::<TouchEmu>(
        &BtnEvent { time, state: ButtonState::Released, button: BTN_RIGHT },
        _loop,
    );
}

/// Drive a canvas finger-pan on the world bus (anchored at the current pointer
/// location, natural-scroll honoured like the trackpad axis path). `momentum`
/// glides (fling/coast); `!momentum` is a strict 1:1 move (2-finger touch pan).
pub fn pan(_loop: &mut compositor_orchestration_core_state_base::Loop, dx: f64, dy: f64, momentum: bool) {
    // Both touch pan settings apply here — the single choke point for every touch
    // pan. `input_touch_pan_speed` scales the gain; `input_touch_linear_pan` (on by
    // default) forces STRICT (no-momentum) pans, so a finger pan tracks 1:1 with no
    // post-release coast regardless of what the call site requested.
    let speed = PAN_SPEED * _loop.inner.preference.input_touch_pan_speed;
    let momentum = momentum && !_loop.inner.preference.input_touch_linear_pan;
    let loc = _loop.state.seat.seat.get_pointer().unwrap().current_location();
    let (dx, dy) = (dx * speed, dy * speed);
    let (h, v) = if _loop.inner.preference.input_natural_scroll { (-dx, -dy) } else { (dx, dy) };
    let ev = compositor_support_system_input_event_base::base::InputEvent::PointerAxis {
        horizontal: h,
        vertical: v,
        x: loc.x,
        y: loc.y,
        finger: true,
        momentum,
        from_touch: true,
    };
    let _ = compositor_orchestration_input_drive_base::drive::route(_loop, ev);
}

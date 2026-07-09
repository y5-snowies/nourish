//! Pointer-emulation primitives. A single finger on the desktop / compositor UI
//! drives the pointer through the very same entry points a mouse uses (so canvas
//! grab, window focus, viewport and overview all behave identically). The session
//! decides *when* to call these; here we only synthesize the pointer events.
use super::backend::{AbsEvent, BtnEvent, TouchEmu};
use smithay::backend::input::{AbsolutePositionEvent, ButtonState, InputBackend};

/// Reference width used to read a touch event's normalized 0..1 position via the
/// backend-agnostic `x_transformed(width)` (both real backends compute it as
/// `normalized * width`, so dividing back out recovers the fraction).
const REF: i32 = 1_000_000;

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
        &BtnEvent { time, state: ButtonState::Pressed },
        _loop,
    );
}

/// Emulate a left-button release.
pub fn release(_loop: &mut compositor_orchestration_core_state_base::Loop, time: u32) {
    crate::button::button::<TouchEmu>(
        &BtnEvent { time, state: ButtonState::Released },
        _loop,
    );
}

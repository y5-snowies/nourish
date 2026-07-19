//! Single-finger pointer emulation for the auxiliary compositor screens — the lock
//! screen and the world picker. These have their own pointer handler sets (not the
//! main desktop's), so touch there just drives *those* handlers: down = move + press,
//! motion = move, up = release. No multi-touch / gestures / wl_touch (they are
//! compositor UI, not client surfaces).
//!
//! Only the BACKEND-GENERIC half lives here. The per-screen adapters that name the
//! lock / picker handlers live in those crates (`lock.seat/seat.input::touch`,
//! `picker.seat/seat.dispatch`), so orchestration does not depend on y5 expansion
//! crates just to host them.
pub use super::backend::{AbsEvent, BTN_LEFT, BtnEvent, TouchEmu};
use super::emulate::fraction;
use smithay::backend::input::{ButtonState, Event, InputBackend};
use compositor_orchestration_core_state_base::Loop;

/// Handler pair for a screen: absolute-motion and button, already instantiated
/// for the `TouchEmu` backend.
pub type Abs = fn(&AbsEvent, &mut Loop);
pub type Btn = fn(&BtnEvent, &mut Loop);

pub fn down<I: InputBackend>(event: &I::TouchDownEvent, _loop: &mut Loop, abs: Abs, btn: Btn) {
    let (nx, ny) = fraction::<I, _>(event);
    let time = event.time_msec();
    abs(&AbsEvent { time, nx, ny }, _loop);
    btn(&BtnEvent { time, state: ButtonState::Pressed, button: BTN_LEFT }, _loop);
}

pub fn motion<I: InputBackend>(event: &I::TouchMotionEvent, _loop: &mut Loop, abs: Abs) {
    let (nx, ny) = fraction::<I, _>(event);
    abs(&AbsEvent { time: event.time_msec(), nx, ny }, _loop);
}

pub fn release<I: InputBackend, E: Event<I>>(event: &E, _loop: &mut Loop, btn: Btn) {
    btn(&BtnEvent { time: event.time_msec(), state: ButtonState::Released, button: BTN_LEFT }, _loop);
}

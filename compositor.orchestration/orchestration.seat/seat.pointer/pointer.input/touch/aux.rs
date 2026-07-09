//! Single-finger pointer emulation for the auxiliary compositor screens — the
//! lock screen and the world picker. These have their own pointer handler sets
//! (not the main desktop's), so touch there just drives *those* handlers: down =
//! move + press, motion = move, up = release. No multi-touch / gestures / wl_touch
//! (they are compositor UI, not client surfaces).
use super::backend::{AbsEvent, BtnEvent, TouchEmu};
use super::emulate::fraction;
use smithay::backend::input::{ButtonState, Event, InputBackend};
use compositor_orchestration_core_state_base::Loop;

/// Handler pair for a screen: absolute-motion and button, already instantiated
/// for the `TouchEmu` backend.
type Abs = fn(&AbsEvent, &mut Loop);
type Btn = fn(&BtnEvent, &mut Loop);

fn down<I: InputBackend>(event: &I::TouchDownEvent, _loop: &mut Loop, abs: Abs, btn: Btn) {
    let (nx, ny) = fraction::<I, _>(event);
    let time = event.time_msec();
    abs(&AbsEvent { time, nx, ny }, _loop);
    btn(&BtnEvent { time, state: ButtonState::Pressed }, _loop);
}

fn motion<I: InputBackend>(event: &I::TouchMotionEvent, _loop: &mut Loop, abs: Abs) {
    let (nx, ny) = fraction::<I, _>(event);
    abs(&AbsEvent { time: event.time_msec(), nx, ny }, _loop);
}

fn release<I: InputBackend, E: Event<I>>(event: &E, _loop: &mut Loop, btn: Btn) {
    btn(&BtnEvent { time: event.time_msec(), state: ButtonState::Released }, _loop);
}

/// Lock screen (`compositor_y5_lock_seat_input`).
pub mod lock {
    use super::*;
    use compositor_y5_lock_seat_input::pointer::{button::button, motion::absolute};
    pub fn down<I: InputBackend>(e: &I::TouchDownEvent, l: &mut Loop) {
        super::down::<I>(e, l, absolute::<TouchEmu>, button::<TouchEmu>);
    }
    pub fn motion<I: InputBackend>(e: &I::TouchMotionEvent, l: &mut Loop) {
        super::motion::<I>(e, l, absolute::<TouchEmu>);
    }
    pub fn up<I: InputBackend>(e: &I::TouchUpEvent, l: &mut Loop) {
        super::release::<I, _>(e, l, button::<TouchEmu>);
    }
}

/// World picker (`compositor_y5_picker_seat_pointer`).
pub mod picker {
    use super::*;
    use compositor_y5_picker_seat_pointer::pointer::{absolute, button};
    pub fn down<I: InputBackend>(e: &I::TouchDownEvent, l: &mut Loop) {
        super::down::<I>(e, l, absolute::<TouchEmu>, button::<TouchEmu>);
    }
    pub fn motion<I: InputBackend>(e: &I::TouchMotionEvent, l: &mut Loop) {
        super::motion::<I>(e, l, absolute::<TouchEmu>);
    }
    pub fn up<I: InputBackend>(e: &I::TouchUpEvent, l: &mut Loop) {
        super::release::<I, _>(e, l, button::<TouchEmu>);
    }
}

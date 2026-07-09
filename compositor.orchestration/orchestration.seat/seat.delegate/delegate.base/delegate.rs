use smithay::backend::input::{
    AbsolutePositionEvent, ButtonState, InputBackend, InputEvent, KeyboardKeyEvent,
    PointerButtonEvent,
};
use compositor_orchestration_core_state_base::Loop;

use crate::{delegate_lock, delegate_main};

/// Delegation of input events from the compositor seat loop
pub fn process_input_event<I: InputBackend>(_loop: &mut Loop, event: &InputEvent<I>) {
    // The world-selection screen owns input while it's the active world (gated on
    // the active world, not Status — it never sets a Status variant).
    if _loop.inner.worlds.active_id() == compositor_y5_picker_system_base::base::PICKER_WORLD {
        // Touch on the picker is single-finger pointer emulation against the
        // picker's own handlers; everything else goes to the picker dispatch.
        use compositor_orchestration_seat_pointer_input::touch::aux::picker;
        match event {
            InputEvent::TouchDown { event, .. } => picker::down::<I>(event, _loop),
            InputEvent::TouchMotion { event, .. } => picker::motion::<I>(event, _loop),
            InputEvent::TouchUp { event, .. } => picker::up::<I>(event, _loop),
            InputEvent::TouchCancel { .. } | InputEvent::TouchFrame { .. } => {}
            _ => compositor_y5_picker_seat_dispatch::dispatch::process_input_event(_loop, event),
        }
        return;
    }
    match _loop.inner.status {
        compositor_orchestration_core_state_base::state::Status::Running => {
            delegate_main::process_input_event(_loop, event);
        }
        compositor_orchestration_core_state_base::state::Status::Locked { pending, .. } => {
            // While running, ignore all inputs
            if pending {
                return;
            }
            delegate_lock::process_input_event(_loop, event);
        }
        compositor_orchestration_core_state_base::state::Status::Sleep { .. } => {
            return;
        }
        compositor_orchestration_core_state_base::state::Status::Terminate => {
            return;
        }
        compositor_orchestration_core_state_base::state::Status::Unlock { .. } => {
            return;
        }
    }
}

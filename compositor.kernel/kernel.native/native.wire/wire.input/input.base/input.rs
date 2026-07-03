//! Input wiring: the libinput source -> compositor lifecycle + redraw
//! scheduling. (Ex wire.rs `start()` libinput closure.)

use compositor_kernel_input_loop_libinput_base::libinput::LibinputSource;
use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use smithay::backend::input::InputEvent;
use smithay::reexports::input::DeviceCapability;
use smithay::reexports::calloop::EventLoop;
use std::cell::RefCell;
use std::rc::Rc;
use compositor_orchestration_core_state_base::state::StatusSession;
use compositor_orchestration_core_state_base::Loop;

pub fn register(
    event_loop: &mut EventLoop<Loop>,
    libinput_source: LibinputSource,
    ctx_rc: Rc<RefCell<NativeRenderContext>>,
) {
    event_loop
        .handle()
        .insert_source(libinput_source, move |event, _, state| {
            if let StatusSession::Paused = state.inner.status_session {
                return;
            }
            // Track physical keyboards so `led_state_changed` can drive their
            // LEDs, and seed each newly added keyboard with the current LED
            // state (e.g. the NumLock-on-by-default set at seat creation).
            match &event {
                InputEvent::DeviceAdded { device }
                    if device.has_capability(DeviceCapability::Keyboard) =>
                {
                    let mut device = device.clone();
                    if let Some(keyboard) = state.state.seat.seat.get_keyboard() {
                        device.led_update(keyboard.led_state().into());
                    }
                    state.state.seat.keyboards.push(device);
                }
                InputEvent::DeviceRemoved { device } => {
                    state.state.seat.keyboards.retain(|d| d != device);
                }
                _ => {}
            }
            // Any input event potentially changes what should be on screen
            // (cursor position, focus, key feedback). Request a redraw.
            // When DARK (no output), only keyboard events run the full pipeline so
            // the always-on fixed shortcuts (VT switch, volume, media) still work;
            // pointer/touch/gesture processing is dropped (no display to interact
            // with — avoids warping the cursor / refocusing against a dead space).
            let dark = *state.inner.kernel.get(
                &compositor_orchestration_driver_lid_base::base::DISPLAY_OFF,
            ) || ctx_rc.borrow().pipe().drm_output.is_none();
            if !dark || matches!(event, InputEvent::Keyboard { .. }) {
                compositor_orchestration_draw_state_lifecycle::lifecycle::input(state, &event);
            }
            // Display request queues (lid apply, settings mode change, activate/
            // deactivate reconcile) and the lock engage are NOT drained here —
            // draining them on the libinput source made them depend on input
            // arriving. They run input-independently via the control-plane ping
            // (`wire.entry`, woken by `ping_control()`, which drains apply / mode /
            // reconcile / lock-engage on its own loop iteration).
            state.schedule_redraw();
        })
        .unwrap();
}

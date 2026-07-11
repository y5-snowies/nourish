//! Input wiring: the libinput source -> compositor lifecycle + redraw
//! scheduling. (Ex wire.rs `start()` libinput closure.)

use compositor_kernel_input_loop_libinput_base::libinput::LibinputSource;
use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use smithay::backend::input::{Event, InputEvent};
use smithay::reexports::input::{Device, DeviceCapability};
use smithay::reexports::calloop::EventLoop;
use std::cell::RefCell;
use std::rc::Rc;
use compositor_orchestration_core_state_base::state::{StatusSession, TouchCursor};
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
            // Per-device libinput settings (tap-to-click); track keyboards (LED
            // mirroring) and touch devices (settings claim list).
            match &event {
                InputEvent::DeviceAdded { device } => {
                    let mut device = device.clone();
                    compositor_kernel_input_libinput_config_base::config::on_device_added(
                        &mut device,
                        &Default::default(),
                    );
                    if device.has_capability(DeviceCapability::Keyboard) {
                        if let Some(keyboard) = state.state.seat.seat.get_keyboard() {
                            device.led_update(keyboard.led_state().into());
                        }
                        state.state.seat.keyboards.push(device);
                    } else if device.has_capability(DeviceCapability::Touch) {
                        state.state.seat.touch_devices.push(device);
                        compositor_kernel_native_wire_input_map::map::write_touch_snapshot(state);
                    }
                }
                InputEvent::DeviceRemoved { device } => {
                    state.state.seat.keyboards.retain(|d| d != device);
                    state.state.seat.touch_devices.retain(|d| d != device);
                    compositor_kernel_native_wire_input_map::map::write_touch_snapshot(state);
                }
                _ => {}
            }
            // A touch must land on the touch device's OWN display: resolve its
            // output via libinput `output_name`, pin the cursor there, else discard.
            let touch_device: Option<Device> = match &event {
                InputEvent::TouchDown { event } => Some(event.device()),
                InputEvent::TouchMotion { event } => Some(event.device()),
                InputEvent::TouchUp { event } => Some(event.device()),
                InputEvent::TouchCancel { event } => Some(event.device()),
                InputEvent::TouchFrame { event } => Some(event.device()),
                _ => None,
            };
            if let Some(device) = touch_device {
                let Some(key) =
                    compositor_kernel_native_wire_input_map::map::touch_output(state, &device)
                else {
                    return;
                };
                // Entering touch mode: hide + snapshot the shared cursor, then hand
                // the single active output to the touched panel.
                state.touch_enter();
                state.inner.cursor_output = Some(key.clone());
                state.inner.output_views_mut().set_current(&key);
            }
            // A real pointer/mouse event ends touch mode: restore the cursor where
            // it was (position + monitor) before this event is routed.
            state.touch_restore_if_pointer(&event);
            // When DARK (no output), only keyboard events run the full pipeline so
            // the always-on fixed shortcuts (VT switch, volume, media) still work;
            // pointer/touch/gesture processing is dropped (no display to interact with).
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

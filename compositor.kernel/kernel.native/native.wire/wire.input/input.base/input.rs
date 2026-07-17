//! Input wiring: the libinput source -> compositor lifecycle + redraw
//! scheduling. (Ex wire.rs `start()` libinput closure.)

use compositor_kernel_input_loop_libinput_base::libinput::LibinputSource;
use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use smithay::backend::input::{Event, InputEvent};
use smithay::reexports::input::event::tablet_pad::{
    ButtonState as PadButtonState, RingAxisSource, StripAxisSource, TabletPadEvent,
};
use smithay::reexports::input::{Device, DeviceCapability};
use smithay::reexports::calloop::EventLoop;
use smithay::wayland::tablet_manager::TabletDescriptor;
use std::cell::RefCell;
use std::rc::Rc;
use compositor_orchestration_core_state_base::state::{StatusSession, TouchCursor};
use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_support_smithay_dispatch_wire_tablet::tablet as tbl;

/// Drive the external `zwp_tablet_pad_v2` senders from a libinput pad event
/// (delivered via smithay's `SpecialEvent`; see the y5 vendor patch). Pads are
/// keyed by device sysname, matching `add_pad`.
fn handle_pad(state: &mut Loop, event: &TabletPadEvent) {
    // Local scope so the input-crate `EventTrait::device`/`TabletPadEventTrait::time`
    // don't collide with smithay's `Event::device` used by the touch code above.
    use smithay::reexports::input::event::{tablet_pad::TabletPadEventTrait, EventTrait};
    let key = event.device().sysname().to_string();
    match event {
        TabletPadEvent::Button(e) => {
            let pressed = e.button_state() == PadButtonState::Pressed;
            state.state.tablet.pad_button(&key, e.button_number(), pressed, e.time());
        }
        TabletPadEvent::Ring(e) => {
            let finger = e.source() == RingAxisSource::Finger;
            state.state.tablet.pad_ring(&key, e.number(), e.position(), finger, e.time());
        }
        TabletPadEvent::Strip(e) => {
            let finger = e.source() == StripAxisSource::Finger;
            state.state.tablet.pad_strip(&key, e.number(), e.position(), finger, e.time());
        }
        TabletPadEvent::Dial(e) => {
            // libinput reports high-res v120 delta (120 units per detent).
            state.state.tablet.pad_dial(&key, e.number(), e.dial_v120() as i32, e.time());
        }
        _ => {}
    }
}

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
                    // A graphics tablet exposes tool and/or pad capabilities on
                    // (usually distinct) evdev nodes; advertise each to clients.
                    // Done first — the keyboard/touch branches below MOVE `device`.
                    if device.has_capability(DeviceCapability::TabletTool)
                        || device.has_capability(DeviceCapability::TabletPad)
                    {
                        let dh = state.inner.loader.display_handle.clone();
                        if device.has_capability(DeviceCapability::TabletTool) {
                            state.state.tablet.add_tablet::<Dispatch>(&dh, &TabletDescriptor::from(&device));
                        }
                        if device.has_capability(DeviceCapability::TabletPad) {
                            let key = device.sysname().to_string();
                            state.state.tablet.add_pad::<Dispatch>(&dh, key, tbl::pad_desc_from_device(&device));
                        }
                    }
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
                    if device.has_capability(DeviceCapability::TabletTool) {
                        state.state.tablet.remove_tablet(&TabletDescriptor::from(device));
                        if !state.state.tablet.has_tablets() {
                            state.state.tablet.clear_tools();
                        }
                    }
                    if device.has_capability(DeviceCapability::TabletPad) {
                        state.state.tablet.remove_pad(&device.sysname());
                    }
                    compositor_kernel_native_wire_input_map::map::write_touch_snapshot(state);
                }
                InputEvent::Special(pad_event) => {
                    // zwp_tablet_pad_v2 events arrive via the SpecialEvent channel.
                    handle_pad(state, pad_event);
                    return;
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

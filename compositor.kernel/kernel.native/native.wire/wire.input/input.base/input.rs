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
            pad_mode(state, &key, e);
            let pressed = e.button_state() == PadButtonState::Pressed;
            // Pen click-to-bind: capture this pad button for the settings tab instead
            // of acting on it.
            if pressed && capture_pad(state, &key, e.button_number()) {
                return;
            }
            // Resolve the user's per-button binding (forwards natively when unbound).
            compositor_orchestration_seat_pointer_input::tablet::pad::button(
                state, &key, e.button_number(), pressed, e.time(),
            );
        }
        TabletPadEvent::Ring(e) => {
            pad_mode(state, &key, e);
            let finger = e.source() == RingAxisSource::Finger;
            state.state.tablet.pad_ring(&key, e.number(), e.position(), finger, e.time());
        }
        TabletPadEvent::Strip(e) => {
            pad_mode(state, &key, e);
            let finger = e.source() == StripAxisSource::Finger;
            state.state.tablet.pad_strip(&key, e.number(), e.position(), finger, e.time());
        }
        TabletPadEvent::Dial(e) => {
            pad_mode(state, &key, e);
            // libinput reports high-res v120 delta (120 units per detent).
            let v120 = e.dial_v120() as i32;
            match state.inner.preference.pen.dial.clone() {
                // Remap: inject a modifier + wheel scroll (e.g. Alt+Wheel for brush
                // size) into the focused client — for apps without tablet-v2.
                compositor_developer_environment_preference_base::base::PenAction::Wheel { mods } => {
                    compositor_orchestration_seat_pointer_input::tablet::inject::wheel(
                        state, &mods, v120, e.time(),
                    );
                }
                // Default: the Hand grab zooms the world; otherwise forward the native
                // dial event to the focused tablet client.
                _ => {
                    if hand_active(state) {
                        dial_zoom(state, v120);
                    } else {
                        state.state.tablet.pad_dial(&key, e.number(), v120, e.time());
                    }
                }
            }
        }
        _ => {}
    }
}

/// Is the Hand (navigation) tool active? Reuses the pen input layer's check, so the
/// dial zooms under BOTH the canvas Hand grab and the touch pane's Hand mode.
fn hand_active(state: &Loop) -> bool {
    compositor_orchestration_seat_pointer_input::tablet::hand_active(state)
}

/// If the settings Pen tab is armed to capture a pad button, record this one and
/// disarm (drained to the UI by the reconciler). Returns `true` when captured.
fn capture_pad(state: &mut Loop, device: &str, button: u32) -> bool {
    use compositor_orchestration_driver_settings_base::base::{PenCapture, SETTINGS, SETTINGS_MUT};
    if state.inner.kernel.get(&SETTINGS).pen_capture != PenCapture::Pad {
        return false;
    }
    let st = state.inner.kernel.get_mut(&SETTINGS_MUT);
    st.pen_capture = PenCapture::None;
    st.pen_captured_pad = Some((device.to_string(), button));
    true
}

/// Zoom the canvas from a dial detent by synthesizing a cursor-anchored mouse-wheel
/// scroll on the world input bus (one detent ≈ one wheel step). The camera system
/// only zooms a non-finger axis while it owns the gesture — which is exactly the
/// Hand grab — so this is gated on `hand_active` by the caller.
fn dial_zoom(state: &mut Loop, v120: i32) {
    use compositor_support_system_input_event_base::base::InputEvent;
    let Some(pointer) = state.state.seat.seat.get_pointer() else { return };
    let loc = pointer.current_location();
    let ev = InputEvent::PointerAxis {
        horizontal: 0.0,
        vertical: v120 as f64 / 120.0,
        x: loc.x,
        y: loc.y,
        finger: false,
        momentum: false,
        from_touch: false,
    };
    let _ = compositor_orchestration_input_drive_base::drive::route(state, ev);
}

/// Announce a pad event's mode-group mode to the focused client (deduped in the
/// sender). Sent before the button/ring/strip/dial event so a mode-toggle button's
/// switch reaches the client ahead of the button that caused it.
fn pad_mode<E>(state: &mut Loop, key: &str, e: &E)
where
    E: smithay::reexports::input::event::tablet_pad::TabletPadEventTrait,
{
    let group = e.mode_group().index() as usize;
    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
    state.state.tablet.pad_mode_switch(key, group, e.mode(), serial, e.time());
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

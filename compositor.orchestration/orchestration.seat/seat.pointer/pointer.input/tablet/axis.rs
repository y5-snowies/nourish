//! `zwp_tablet_tool_v2` axis events (motion + pressure/distance/tilt/rotation/
//! slider/wheel). Changed axes are queued and flushed on the next motion (protocol
//! requirement). If the stroke is `Pan`, the pen drives the canvas instead of a
//! client; otherwise motion + axes forward to the tablet-aware surface under it.

use smithay::backend::input::{Event, InputBackend, TabletToolAxisEvent, TabletToolEvent};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::tablet_manager::TabletDescriptor;
use compositor_orchestration_core_state_base::Loop;
use crate::tablet::{client, coords, cursor};

pub fn axis<I: InputBackend>(event: &I::TabletToolAxisEvent, _loop: &mut Loop) {
    let tool = event.tool();
    let device = event.device();
    let tablet = TabletDescriptor::from(&device);
    let time = event.time_msec();
    let (_, world) = coords::world::<I, _>(event, _loop);

    if event.pressure_has_changed() {
        _loop.state.tablet.tool_pressure(&tool, event.pressure());
    }
    if event.distance_has_changed() {
        _loop.state.tablet.tool_distance(&tool, event.distance());
    }
    if event.tilt_has_changed() {
        _loop.state.tablet.tool_tilt(&tool, event.tilt());
    }
    if event.rotation_has_changed() {
        _loop.state.tablet.tool_rotation(&tool, event.rotation());
    }
    if event.slider_has_changed() {
        _loop.state.tablet.tool_slider(&tool, event.slider_position());
    }
    if event.wheel_has_changed() {
        _loop
            .state
            .tablet
            .tool_wheel(&tool, event.wheel_delta(), event.wheel_delta_discrete());
    }

    // Cursor follows the pen, then native tool motion (+ queued axes) is forwarded
    // to the tablet-aware surface under it (or `None` ⇒ the tool leaves it).
    cursor::follow(_loop, world, time);
    let focus = client::tablet_focus(_loop, world);
    let serial = SERIAL_COUNTER.next_serial();
    _loop
        .state
        .tablet
        .tool_motion(&tool, &tablet, world, focus, serial, time);
}

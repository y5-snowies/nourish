//! `zwp_tablet_tool_v2` axis events (motion + pressure/distance/tilt/rotation/
//! slider/wheel). Changed axes are queued and flushed on the next motion (protocol
//! requirement); motion + axes forward to the tablet-aware surface under the pen.
//! Under the Hand or Select tool the pen drives the CANVAS (glide-pan / rubber-band)
//! via the pointer bus and forwards nothing to tablet clients.
//!
//! In **pressure pen-down** mode (`below_threshold_cursor`) the tip is derived HERE
//! from pressure crossing `tip_threshold` — the driver's own tip event is ignored, so
//! a tablet that reports "pen down" on mere detection / max distance only registers a
//! real press. Below the threshold the pen hovers (cursor + hover forwarding); above
//! it, `start_draw` runs the normal stroke (draw on a tablet app, click on a window,
//! inert on bare canvas).

use smithay::backend::input::{Event, InputBackend, TabletToolAxisEvent, TabletToolEvent};
use smithay::utils::SERIAL_COUNTER;
use smithay::input::tablet::TabletDescriptor;
use compositor_support_smithay_dispatch_wire_tablet::tablet::Stroke;
use compositor_orchestration_core_state_base::Loop;
use crate::tablet::{client, coords, cursor, hand_active, select_active, tip};

pub fn axis<I: InputBackend>(event: &I::TabletToolAxisEvent, _loop: &mut Loop) {
    let tool = event.tool();
    let device = event.device();
    let tablet = TabletDescriptor::from(&device);
    let time = event.time_msec();
    let (screen, world) = coords::world::<I, _>(event, _loop);

    // Hand / Select tools: the pen navigates the canvas via the pointer bus, not
    // tablet events. Cursor tracks the pen; in a Pan stroke, glide-pan by the pen's
    // physical-screen delta (like the touch Hand tool's finger glide).
    if hand_active(_loop) || select_active(_loop) {
        let last = _loop.state.tablet.last_pen_screen;
        _loop.state.tablet.last_pen_screen = Some(screen);
        cursor::follow(_loop, screen, world, time);
        if _loop.state.tablet.stroke == Stroke::Pan {
            if let Some(last) = last {
                crate::touch::emulate::pan(_loop, screen.x - last.x, screen.y - last.y, false);
            }
        }
        return;
    }

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

    // Cursor follows the pen (and drives the canvas via the bus).
    cursor::follow(_loop, screen, world, time);

    // Pressure pen-down: begin/end the stroke as pressure crosses the threshold,
    // overriding the driver's (possibly broken) tip detection.
    if _loop.inner.preference.pen.below_threshold_cursor {
        let above = event.pressure() >= _loop.inner.preference.pen.tip_threshold as f64;
        let down = _loop.state.tablet.stroke != Stroke::None;
        if above && !down {
            tip::start_draw(_loop, &tool, screen, world, time);
        } else if !above && down {
            tip::end_draw(_loop, &tool, time);
        }
    }

    // Native tool motion (+ queued axes) forwards to the tablet-aware surface under
    // the pen (hover while not drawing; the live stroke while drawing).
    let focus = client::tablet_focus(_loop, world);
    let serial = SERIAL_COUNTER.next_serial();
    _loop
        .state
        .tablet
        .tool_motion(&tool, &tablet, world, focus, serial, time);
}

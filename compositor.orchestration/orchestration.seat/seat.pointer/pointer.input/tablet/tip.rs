//! `zwp_tablet_tool_v2` tip down/up — the stroke boundary. The role is latched at
//! tip-down from the active tool:
//!   * **Hand** (canvas grab or touch pane) → `Pan`: the tip-drag glide-pans the world.
//!   * **Select** (touch pane) → arm the transient select grab + press → rubber-band.
//!   * otherwise → `Tablet` draw over a tablet-aware window, else `Pointer` (a click),
//!     or, in below-threshold cursor mode, deferred to a pressure crossing (axis.rs).
//! Mirrors how `touch/session.rs` maps `TouchMode` onto the same canvas mechanisms.

use smithay::backend::input::{Event, InputBackend, TabletToolDescriptor, TabletToolEvent, TabletToolTipEvent, TabletToolTipState};
use smithay::utils::{Logical, Physical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::export::{CanvasGrab, TargetOption};
use compositor_support_smithay_dispatch_wire_tablet::tablet::Stroke;
use crate::tablet::{client, coords, cursor, hand_active, select_active};

/// Begin a stroke at `world` (`screen` = the physical cursor position): a native tool
/// tip on a tablet-aware surface, else the pointer path (a click at the cursor). In
/// light-touch mode the pen is a pure cursor, so this ALWAYS takes the pointer path —
/// no `zwp_tablet_tool_v2` tip is ever emitted. Only reached outside the Hand/Select
/// tools (those latch their own role in `tip`).
pub fn start_draw(
    _loop: &mut Loop,
    tool: &TabletToolDescriptor,
    screen: Point<f64, Physical>,
    world: Point<f64, Logical>,
    time: u32,
) {
    let light_touch = _loop.inner.preference.pen.below_threshold_cursor;
    if !light_touch && client::tablet_focus(_loop, world).is_some() {
        client::apply_focus(_loop, world);
        let serial = SERIAL_COUNTER.next_serial();
        _loop.state.tablet.tool_tip_down(tool, serial, time);
        _loop.state.tablet.stroke = Stroke::Tablet;
    } else {
        cursor::follow(_loop, screen, world, time);
        crate::touch::emulate::press(_loop, time);
        _loop.state.tablet.stroke = Stroke::Pointer;
    }
}

/// End the latched stroke (no-op when none / a pan is active — the pan just stops).
pub fn end_draw(_loop: &mut Loop, tool: &TabletToolDescriptor, time: u32) {
    match _loop.state.tablet.stroke {
        Stroke::Tablet => _loop.state.tablet.tool_tip_up(tool, time),
        Stroke::Pointer => crate::touch::emulate::release(_loop, time),
        Stroke::Pan | Stroke::None => {}
    }
    _loop.state.tablet.stroke = Stroke::None;
}

/// Clear the transient Select grab on lift, mirroring `touch/session.rs::disarm_select`.
fn disarm_select(_loop: &mut Loop) {
    if matches!(_loop.inner.canvas().Grab, CanvasGrab::Target(TargetOption::Select { .. })) {
        _loop.inner.canvas_mut().Grab = CanvasGrab::None;
    }
}

pub fn tip<I: InputBackend>(event: &I::TabletToolTipEvent, _loop: &mut Loop) {
    let tool = event.tool();
    let time = event.time_msec();
    let (screen, world) = coords::world::<I, _>(event, _loop);
    match event.tip_state() {
        TabletToolTipState::Down => {
            _loop.state.tablet.phys_tip = true;
            _loop.state.tablet.last_pen_screen = Some(screen);
            if hand_active(_loop) {
                // Hand tool: the tip-drag pans (axis.rs); no draw, no click.
                cursor::follow(_loop, screen, world, time);
                _loop.state.tablet.stroke = Stroke::Pan;
            } else if select_active(_loop) {
                // Select tool: arm the transient grab (disarmed on lift) so the
                // emulated press + drag rubber-band selects, like the touch session.
                _loop.inner.canvas_mut().Grab = CanvasGrab::Target(TargetOption::Select { Append: true });
                cursor::follow(_loop, screen, world, time);
                crate::touch::emulate::press(_loop, time);
                _loop.state.tablet.stroke = Stroke::Pointer;
            } else if _loop.inner.preference.pen.below_threshold_cursor {
                // Light-touch mode: the click is deferred to a pressure crossing
                // (axis.rs); a light touch just moves the cursor.
                cursor::follow(_loop, screen, world, time);
            } else {
                start_draw(_loop, &tool, screen, world, time);
            }
        }
        TabletToolTipState::Up => {
            _loop.state.tablet.phys_tip = false;
            end_draw(_loop, &tool, time);
            disarm_select(_loop);
        }
    }
}

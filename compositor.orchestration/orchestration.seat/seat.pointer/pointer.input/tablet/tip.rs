//! `zwp_tablet_tool_v2` tip down/up — the stroke boundary. The role is latched at
//! tip-down from the active tool:
//!   * **Hand** (canvas grab or touch pane) → `Pan`: the tip-drag glide-pans the world.
//!   * **Select** (touch pane) → arm the transient select grab + press → rubber-band.
//!   * **Light touch** → a pure hovering cursor: the tip never presses (clicks come
//!     only from the barrel buttons).
//!   * otherwise → `Tablet` draw over a tablet-aware window, a `Pointer` click over any
//!     other window / iced overlay, or NOTHING over bare canvas (a plain pen tip must
//!     not start a canvas pan — that's the Hand tool's job — so a spurious pen-down
//!     from a bad driver can't pan).
//! Mirrors how `touch/session.rs` maps `TouchMode` onto the same canvas mechanisms.

use smithay::backend::input::{Event, InputBackend, TabletToolDescriptor, TabletToolEvent, TabletToolTipEvent, TabletToolTipState};
use smithay::utils::{Logical, Physical, Point, SERIAL_COUNTER};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::export::{CanvasGrab, TargetOption};
use compositor_support_smithay_dispatch_wire_tablet::tablet::Stroke;
use crate::tablet::{client, coords, cursor, hand_active, select_active};

/// Begin a stroke at `world` (`screen` = the physical cursor position): a native tool
/// tip on a tablet-aware surface; else a `Pointer` click over any other window / iced
/// overlay (the pen as a mouse); else — over bare canvas — NOTHING, so a plain pen tip
/// never initiates a canvas pan. Only reached outside the Hand/Select/Light-touch modes.
pub fn start_draw(
    _loop: &mut Loop,
    tool: &TabletToolDescriptor,
    screen: Point<f64, Physical>,
    world: Point<f64, Logical>,
    time: u32,
) {
    if client::tablet_focus(_loop, world).is_some() {
        client::apply_focus(_loop, world);
        let serial = SERIAL_COUNTER.next_serial();
        _loop.state.tablet.tool_tip_down(tool, serial, time);
        _loop.state.tablet.stroke = Stroke::Tablet;
    } else if client::topmost(_loop, world).is_some() {
        // A window / layer / iced overlay (NOT bare canvas) → the pen acts as a mouse.
        cursor::follow(_loop, screen, world, time);
        crate::touch::emulate::press(_loop, time);
        _loop.state.tablet.stroke = Stroke::Pointer;
    } else {
        // Bare canvas → inert: no press means no canvas pan from a plain pen tip.
        _loop.state.tablet.stroke = Stroke::None;
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
            _loop.state.tablet.last_pen_screen = Some(screen);
            // A pen tip OUTSIDE the open launcher dismisses it, like touch (a tip ON it
            // falls through so a cell can launch). Mouse never reaches this path.
            let hit_handle = client::topmost(_loop, world).and_then(|h| h.iced_handle());
            if compositor_y5_launcher_interface_base::interface::dismiss_if_outside(_loop, hit_handle) {
                return;
            }
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
                // Light touch: a pure hovering cursor — the tip never presses.
                cursor::follow(_loop, screen, world, time);
            } else {
                start_draw(_loop, &tool, screen, world, time);
            }
        }
        TabletToolTipState::Up => {
            end_draw(_loop, &tool, time);
            disarm_select(_loop);
        }
    }
}

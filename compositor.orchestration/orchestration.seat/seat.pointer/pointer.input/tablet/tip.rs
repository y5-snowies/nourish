//! `zwp_tablet_tool_v2` tip down/up — the stroke boundary. The stroke role is
//! latched here at tip-down (Client over a tablet-aware window, Pan over the
//! canvas, or inert over a non-tablet window) and held until tip-up.

use smithay::backend::input::{Event, InputBackend, TabletToolEvent, TabletToolTipEvent, TabletToolTipState};
use smithay::utils::SERIAL_COUNTER;
use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_dispatch_wire_tablet::tablet::Stroke;
use crate::tablet::{client, coords, cursor};

pub fn tip<I: InputBackend>(event: &I::TabletToolTipEvent, _loop: &mut Loop) {
    let tool = event.tool();
    let time = event.time_msec();
    let (_, world) = coords::world::<I, _>(event, _loop);
    match event.tip_state() {
        TabletToolTipState::Down => {
            if client::tablet_focus(_loop, world).is_some() {
                // Tablet-aware surface → native tool tip.
                client::apply_focus(_loop, world);
                let serial = SERIAL_COUNTER.next_serial();
                _loop.state.tablet.tool_tip_down(&tool, serial, time);
                _loop.state.tablet.stroke = Stroke::Tablet;
            } else {
                // Everything else (non-tablet window / canvas) → the pen acts as a
                // mouse: click at the cursor (which already tracks the pen).
                cursor::follow(_loop, world, time);
                crate::touch::emulate::press(_loop, time);
                _loop.state.tablet.stroke = Stroke::Pointer;
            }
        }
        TabletToolTipState::Up => {
            match _loop.state.tablet.stroke {
                Stroke::Tablet => _loop.state.tablet.tool_tip_up(&tool, time),
                Stroke::Pointer => crate::touch::emulate::release(_loop, time),
                Stroke::None => {}
            }
            _loop.state.tablet.stroke = Stroke::None;
        }
    }
}

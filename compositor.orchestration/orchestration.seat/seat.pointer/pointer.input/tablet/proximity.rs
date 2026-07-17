//! `zwp_tablet_tool_v2` proximity in/out. Registers the tablet + tool lazily (both
//! idempotent), then — on `In` over a tablet-aware window — forwards `proximity_in`
//! so the client gets hover (brush preview / hover cursor). Hover is never a pan.

use smithay::backend::input::{
    Event, InputBackend, ProximityState, TabletToolEvent, TabletToolProximityEvent,
};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::tablet_manager::TabletDescriptor;
use compositor_orchestration_core_state_base::Loop;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_support_smithay_dispatch_wire_tablet::tablet::Stroke;
use crate::tablet::{client, coords, cursor};

pub fn proximity<I: InputBackend>(event: &I::TabletToolProximityEvent, _loop: &mut Loop) {
    let tool = event.tool();
    let device = event.device();
    let tablet = TabletDescriptor::from(&device);
    let time = event.time_msec();
    let dh = _loop.inner.loader.display_handle.clone();
    _loop.state.tablet.add_tablet::<Dispatch>(&dh, &tablet);
    _loop.state.tablet.add_tool::<Dispatch>(&dh, &tool);

    let (_, world) = coords::world::<I, _>(event, _loop);
    match event.state() {
        ProximityState::In => {
            // Position the cursor at the pen, then forward hover to a tablet-aware
            // surface (brush preview / hover cursor).
            cursor::follow(_loop, world, time);
            if let Some(focus) = client::tablet_focus(_loop, world) {
                let serial = SERIAL_COUNTER.next_serial();
                _loop
                    .state
                    .tablet
                    .tool_proximity_in(&tool, &tablet, world, focus, serial, time);
            }
        }
        ProximityState::Out => {
            _loop.state.tablet.tool_proximity_out(&tool, time);
            _loop.state.tablet.stroke = Stroke::None;
        }
    }
}

//! `zwp_tablet_tool_v2` stylus buttons — forwarded to the surface the tool is
//! focused on (the tool handle no-ops when there is no focus).

use smithay::backend::input::{Event, InputBackend, TabletToolButtonEvent, TabletToolEvent};
use smithay::utils::SERIAL_COUNTER;
use compositor_orchestration_core_state_base::Loop;

pub fn button<I: InputBackend>(event: &I::TabletToolButtonEvent, _loop: &mut Loop) {
    let tool = event.tool();
    let serial = SERIAL_COUNTER.next_serial();
    _loop.state.tablet.tool_button(
        &tool,
        event.button(),
        event.button_state(),
        serial,
        event.time_msec(),
    );
}

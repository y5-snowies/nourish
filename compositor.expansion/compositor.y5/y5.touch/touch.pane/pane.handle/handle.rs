//! Compositor-side handling of touch-pane taps (drained from the surface channel
//! by the global pump). Sets the sticky tool-mode (arming/clearing the canvas
//! Select tool) and opens Overview / World Picker.

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_gesture_touch::touch::TouchMode;
use compositor_y5_touch_pane_view::{PaneMode, TouchPaneMessage};

pub fn delegate(state: &mut Loop, message: TouchPaneMessage) {
    match message {
        TouchPaneMessage::SetMode(m) => set_mode(state, mode_of(m)),
        // Overview / World Picker TOGGLE (open if closed, close if open) and the pane
        // stays visible + on top, so the same button closes them again. The pane
        // out-stacks the overview (it draws in the higher ICED_SCREEN band) and its
        // taps route to it (Screen-iced is hit-tested first).
        TouchPaneMessage::OpenOverview => {
            compositor_y5_overview_interface_base::base::toggle(state);
        }
        TouchPaneMessage::OpenWorldPicker => {
            compositor_y5_picker_interface_entry::entry::toggle(state);
        }
        // Close button: hide the pane in this world.
        TouchPaneMessage::Close => {
            state.inner.touch.pane_world = None;
        }
        TouchPaneMessage::SetActive(_) => {}
    }
}

fn mode_of(m: PaneMode) -> TouchMode {
    match m {
        PaneMode::Touch => TouchMode::Touch,
        PaneMode::Pointer => TouchMode::Pointer,
        PaneMode::Hand => TouchMode::Hand,
        PaneMode::Select => TouchMode::Select,
    }
}

/// Switch the sticky touch tool-mode. The tool-modes are TOUCH-EXCLUSIVE — none of
/// them arms a persistent canvas grab, so the MOUSE (which shares the canvas grab) is
/// never put into Hand/Select by the touch pane. Select just flags `select_visual`
/// (draws the selection frame); the touch `session` arms the real Select grab only
/// for the span of a touch sequence. Hand pans via forced glide in the session.
fn set_mode(state: &mut Loop, mode: TouchMode) {
    state.inner.canvas_mut().select_visual = mode == TouchMode::Select;
    // Hand tool: let the canvas own wheel/pinch/dial so the tablet dial + mouse wheel
    // zoom (the camera can't see the touch tracker's tool_mode, so mirror it here).
    state.inner.canvas_mut().hand_touch = mode == TouchMode::Hand;
    state.inner.touch.tool_mode = mode;
}

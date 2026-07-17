//! Compositor-side handling of touch-pane taps (drained from the surface channel
//! by the global pump). Sets the sticky tool-mode (arming/clearing the canvas
//! Select tool) and opens Overview / World Picker.

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::export::{ActiveOption, CanvasGrab, TargetOption};
use compositor_orchestration_seat_gesture_touch::touch::TouchMode;
use compositor_y5_touch_pane_view::{PaneMode, TouchPaneMessage};

pub fn delegate(state: &mut Loop, message: TouchPaneMessage) {
    match message {
        TouchPaneMessage::SetMode(m) => set_mode(state, mode_of(m)),
        TouchPaneMessage::OpenOverview => {
            // Hide the pane so it doesn't sit over the overlay, then open it.
            state.inner.touch.pane_world = None;
            compositor_y5_overview_interface_base::base::toggle(state);
        }
        TouchPaneMessage::OpenWorldPicker => {
            state.inner.touch.pane_world = None;
            compositor_y5_picker_interface_entry::entry::request_open(state);
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

/// Switch the sticky tool-mode by arming the matching canvas grab, so touch uses
/// the SAME canvas machinery as the keyboard tools:
///   * `Select` → the Select tool (always append): a touch press selects windows
///     / drags a select-box.
///   * `Hand` → the canvas Hand grab: a touch press+drag pans the camera 1:1 and
///     ignores window hit-testing/focus, exactly like the keyboard Hand tool.
///   * `Touch` / `Pointer` → no armed tool (plain pointer emulation).
fn set_mode(state: &mut Loop, mode: TouchMode) {
    state.inner.canvas_mut().Grab = match mode {
        TouchMode::Select => CanvasGrab::Target(TargetOption::Select { Append: true }),
        TouchMode::Hand => CanvasGrab::Active(ActiveOption::Hand),
        _ => CanvasGrab::None,
    };
    state.inner.touch.tool_mode = mode;
}

use compositor_orchestration_core_state_base::Loop;
use compositor_remote_message_client_base::bind::debug::{RequestNumeric, ResponseNumeric};
use compositor_y5_action_canvas_layout::LayoutFlags;

pub fn numeric(request: RequestNumeric, state: &mut Loop) -> ResponseNumeric {
    // Button 3: Equalize spacing on a column — even gaps, no resizing.
    let BTN_3_DISTRIBUTE_COLUMN: LayoutFlags = LayoutFlags::DISTRIBUTE_VERTICALLY
        | LayoutFlags::ALIGN
        | LayoutFlags::ALIGN_CENTER_HORIZONTAL;

    // Button 4: Snap to clusters (Align_Close on both axes, both edges).
    // Tight clusters stay where they are; outliers come to them.
    let BTN_4_SNAP_TO_CLUSTERS: LayoutFlags = LayoutFlags::ALIGN
        | LayoutFlags::ALIGN_CLOSE
        | LayoutFlags::ALIGN_LEFT
        | LayoutFlags::ALIGN_TOP;

    // Button 5: Stack tightly on focused window (same x and width).
    let BTN_5_COLUMN_TO_FOCUSED: LayoutFlags =
        LayoutFlags::ALIGN | LayoutFlags::ALIGN_LEFT | LayoutFlags::ALIGN_RIGHT;

    // Button 6: Centered overlay. Bare ALIGN = center-to-center.
    let BTN_6_CENTER_ON_FOCUSED: LayoutFlags = LayoutFlags::ALIGN;

    let number = request.number;
    match number {
        // 1 and 2 are handled in the navigator service.
        3 => {
            // Align + Distribute with all options
            compositor_y5_action_canvas_layout::commit(
                state,
                LayoutFlags::ALIGN | LayoutFlags::ALIGN_TOP,
            );
        }
        4 => {
            // Align + Distribute with all options
            compositor_y5_action_canvas_layout::commit(
                state,
                LayoutFlags::ALIGN
                    | LayoutFlags::ALIGN_TOP
                    | LayoutFlags::ALIGN_BOTTOM
                    | LayoutFlags::ALIGN_CLOSE,
            );
        }
        5 => {
            // Just distribute
            compositor_y5_action_canvas_layout::commit(
                state,
                LayoutFlags::DISTRIBUTE_HORIZONTALLY | LayoutFlags::DISTRIBUTE_TARGET_H_AXIS,
            );
        }
        6 => {
            // Vertical stack assuming common width
            compositor_y5_action_canvas_layout::commit_all(
                state,
                &[
                    // Step 1: stack vertically with 0 spacing, center horizontally
                    LayoutFlags::DISTRIBUTE_VERTICALLY
                        | LayoutFlags::DISTRIBUTE_TARGET_V_START
                        | LayoutFlags::ALIGN
                        | LayoutFlags::ALIGN_CENTER_HORIZONTAL
                        | LayoutFlags::ALIGN_CLOSE,
                    // Step 2: stretch all to span the (now collapsed) horizontal bbox
                    LayoutFlags::ALIGN
                        | LayoutFlags::ALIGN_LEFT
                        | LayoutFlags::ALIGN_RIGHT
                        | LayoutFlags::ALIGN_CLOSE,
                ],
            );
        }
        7 => {
            // Stacks the windows vertically. Better with START distribution.
            compositor_y5_action_canvas_layout::commit(state, BTN_3_DISTRIBUTE_COLUMN);
        }
        8 => {
            compositor_y5_action_canvas_layout::commit(state, BTN_4_SNAP_TO_CLUSTERS);
        }
        9 => {
            compositor_y5_action_canvas_layout::commit(state, BTN_5_COLUMN_TO_FOCUSED);
        }
        10 => {
            compositor_y5_action_canvas_layout::commit(state, BTN_6_CENTER_ON_FOCUSED);
        }
        16 => {}
        _ => {}
    }

    ResponseNumeric {}
}

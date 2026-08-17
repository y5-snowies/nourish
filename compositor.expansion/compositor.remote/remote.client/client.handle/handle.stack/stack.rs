use compositor_y5_action_canvas_layout::LayoutFlags;

/// Horizontal stack assuming common height.
pub fn horizontal() -> Vec<LayoutFlags> {
    vec![
        // Step 1: stack horizontally with 0 spacing, center vertically
        LayoutFlags::DISTRIBUTE_HORIZONTALLY
            | LayoutFlags::DISTRIBUTE_TARGET_H_START
            | LayoutFlags::ALIGN
            | LayoutFlags::ALIGN_CENTER_VERTICAL
            | LayoutFlags::ALIGN_CLOSE,
        // Step 2: stretch all to span the (now collapsed) vertical bbox
        LayoutFlags::ALIGN
            | LayoutFlags::ALIGN_TOP
            | LayoutFlags::ALIGN_BOTTOM
            | LayoutFlags::ALIGN_CLOSE,
    ]
}

/// Vertical stack assuming common width.
pub fn vertical() -> Vec<LayoutFlags> {
    vec![
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
    ]
}

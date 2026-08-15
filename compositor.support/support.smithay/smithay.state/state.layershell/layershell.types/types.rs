#[derive(Debug, Clone)]
pub struct OverlayPlacement {
    pub anchor_mode: AnchorMode,
    pub margin: i32,
}

#[derive(Debug, Clone, Copy)]
pub enum AnchorMode {
    BottomCenter,
    TopCenter,
    FollowCursor { offset_x: i32, offset_y: i32 },
    Free { x: i32, y: i32 },
}

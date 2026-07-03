use compositor_y5_canvas_input_state::state::CanvasGrab;

// Selection -> SelectSystem (SELECT); grouping -> GroupSystem (GROUP). Canvas is
// being decomposed into per-system slots; only the input Grab remains here.
pub struct CanvasState {
    pub Grab: CanvasGrab,
    /// Pan-in-progress flag: set when a canvas pan starts, cleared on release;
    /// canvas motion gates camera panning on it. Lives here (with the grab) not
    /// in the camera, so the canvas owner can flip it synchronously.
    pub position_updating: bool,
    /// Momentary hand tool: true while Super is held alone. Touchpad pan/pinch and
    /// the mouse wheel (zoom) own the canvas while set — but NOT the mouse click,
    /// which stays the Move tool that shares the Super modifier. Tracked here —
    /// separate from `Grab` — so the two never entangle; the keyboard handler flips
    /// it, the camera reads it.
    pub finger_pan: bool,
}

impl CanvasState {
    pub fn new() -> Self {
        Self {
            Grab: CanvasGrab::None,
            position_updating: false,
            finger_pan: false,
        }
    }

    /// A canvas/world pointer drag is in progress (window/pane move, scale,
    /// select-box, hand). During these the cursor may cross monitors (grab-to-move a
    /// window between them), so teleport must stay enabled — unlike a screen-surface
    /// drag (settings layout-canvas pan) where it's suppressed.
    pub fn active_grab(&self) -> bool {
        matches!(self.Grab, CanvasGrab::Active(_))
    }
}

use compositor_y5_canvas_input_state::state::CanvasGrab;
use compositor_support_system_input_event_base::base::Modality;

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
    /// True while a Move/Scale grab was started by dragging the SELECT tool's
    /// bounding rect (a corner handle → resize, the interior → move) rather than the
    /// Move/Scale tools. The release restores the Select tool (not Move/Scale) so the
    /// touch Select mode stays sticky, and a no-drag tap toggles selection instead.
    /// Also gates drawing the persistent selection frame during the drag.
    pub select_transform: bool,
    /// Touch Select tool-mode is active (set by the touch pane, touch-only). Drives
    /// drawing the persistent selection frame + handles WITHOUT arming a global canvas
    /// grab, so the mouse — which shares [`Grab`] — is never put into Select mode by a
    /// touch tool-mode. The actual Select grab is armed only for the span of a touch
    /// sequence (see the touch `session`).
    pub select_visual: bool,
    /// Touch Hand tool-mode is active (set by the touch pane). Makes the canvas own
    /// wheel/pinch/dial gestures (so the tablet DIAL and the mouse wheel zoom) while
    /// the touch Hand tool is selected, without arming a global [`Grab`]. Mirrored
    /// here because the camera reads world storage, not the touch tracker.
    pub hand_touch: bool,
    /// Device class of the most recent input event, mirrored from the seat's touch
    /// tracker (the rim writes it in the seat delegate). Systems that read storage —
    /// the camera's gesture-ownership test, the canvas press/release paths — gate
    /// direct-only behaviour on this, because touch and pen arrive as EMULATED
    /// pointer events and are otherwise indistinguishable from a mouse click.
    pub input_modality: Modality,
}

impl CanvasState {
    pub fn new() -> Self {
        Self {
            Grab: CanvasGrab::None,
            position_updating: false,
            finger_pan: false,
            select_transform: false,
            select_visual: false,
            hand_touch: false,
            input_modality: Modality::default(),
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

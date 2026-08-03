//! Camera / sphere / cell tunables for the world-selection scene. Kept free of
//! bevy types so the compositor-side picker (silhouette test, navigation) can
//! share the same numbers.

/// Cube-sphere subdivision: 6 faces × `CELLS_PER_FACE²` cubic cells. Must match
/// `compositor_y5_picker_state_base::base::CELLS_PER_FACE`.
pub const CELLS_PER_FACE: usize = 3;
pub const CELL_COUNT: usize = 6 * CELLS_PER_FACE * CELLS_PER_FACE;

/// Sphere the cells sit on (cell centers are at this radius).
pub const SPHERE_RADIUS: f32 = 1.0;

/// Picker entry transition, in two strictly sequential halves that meet on a
/// fully black frame: the world being left ramps to opaque over `FADE_OUT_SECS`,
/// the switch happens there, and the picker clears the same overlay over
/// `FADE_SECS`. No morph.
pub const FADE_OUT_SECS: f32 = 0.125;
pub const FADE_SECS: f32 = 0.25;

/// Camera: distance from origin + vertical field of view (radians, ~45°).
pub const CAMERA_DISTANCE: f32 = 3.4;
pub const CAMERA_FOV_RAD: f32 = 0.7853982;

/// Idle camera animation: a gentle sway (no sphere spin). Amplitude in world
/// units, speed in rad/s.
pub const SWAY_SPEED: f32 = 0.5;
pub const SWAY_AMPLITUDE: f32 = 0.10;

/// Drag-to-rotate sensitivity: radians of sphere rotation per unit of
/// normalized pointer drag.
pub const ROTATE_SENSITIVITY: f32 = 3.0;

/// Arrow-key navigation turns the view in `NAV_STEP` radian increments, up to
/// `NAV_REACH`, and takes the first cell that reaches the screen centre. One
/// fixed step cannot do it — cube-sphere cells subtend different angles by where
/// on their face they sit, so a single size both skips and stalls. `NAV_REACH`
/// is generous because the sweep stops the moment the centred cell changes: a
/// few iterations on an ordinary press, and it only matters near a pole, where
/// a yaw turn is foreshortened by `cos(pitch)`.
pub const NAV_STEP: f32 = 0.02;
pub const NAV_REACH: f32 = std::f32::consts::PI;

/// Pitch is clamped to ±PITCH_MAX (radians) so the globe never tumbles over a
/// pole and screen-up stays sphere-up; yaw is free. A quarter turn, exactly
/// enough to bring the polar faces to the camera — less would strand cells.
pub const PITCH_MAX: f32 = std::f32::consts::FRAC_PI_2;

/// The refresh rate the two rates below are quoted against. They are applied
/// per SECOND via this exponent (`orient::approach`/`momentum`), so the globe
/// coasts and glides identically at 60 Hz and 240 Hz; a frame-counted rate made
/// a 240 Hz session spin four times as fast.
pub const REFERENCE_HZ: f32 = 60.0;

/// Drag-release momentum: the fraction of the spin velocity retained after one
/// `REFERENCE_HZ` frame, until it settles.
pub const SPIN_DECAY: f32 = 0.94;

/// Selection re-face animation: fraction of the REMAINING angle the orientation
/// slerps toward the target in one `REFERENCE_HZ` frame (so arrow nav glides to
/// the chosen cell instead of snapping).
pub const APPROACH_RATE: f32 = 0.22;

/// Scroll-to-zoom: camera distance = CAMERA_DISTANCE / zoom. Step per axis tick,
/// clamped to [ZOOM_MIN, ZOOM_MAX].
pub const ZOOM_STEP: f32 = 0.12;
pub const ZOOM_MIN: f32 = 0.6;
pub const ZOOM_MAX: f32 = 2.0;

/// The camera's distance from the origin at a given zoom. The ONE definition:
/// the render camera (`three.apply/idle_camera`) and the compositor-side ray
/// cast (`pick.base`) must agree, or clicks land on the cell the sphere shows at
/// some OTHER zoom (they did — picking assumed the un-zoomed distance).
pub fn camera_distance(zoom: f32) -> f32 {
    CAMERA_DISTANCE / zoom.max(0.1)
}

/// Cell square edge as a fraction of the per-face cell pitch (rest is gap).
pub const CELL_FILL: f32 = 0.86;

/// Colours (linear RGBA). Empty cells are transparent (no fill) — only the
/// wireframe shows. Occupied cells carry their thumbnail.
pub const WIRE_COLOR: [f32; 4] = [0.45, 0.50, 0.60, 0.55];
pub const OUTLINE_COLOR: [f32; 4] = [0.55, 0.80, 1.00, 1.0];
pub const PLUS_COLOR: [f32; 4] = [0.85, 0.92, 1.00, 0.95];
/// Fill for a cell that holds a world but has no thumbnail yet (e.g. restored
/// from disk before that world is next active) — a solid patch so the cell reads
/// as occupied instead of empty.
pub const OCCUPIED_COLOR: [f32; 4] = [0.30, 0.38, 0.50, 0.70];

/// "+" glyph on the selected cell: bar length + thickness as fractions of the
/// cell edge.
pub const PLUS_LEN: f32 = 0.5;
pub const PLUS_THICK: f32 = 0.10;

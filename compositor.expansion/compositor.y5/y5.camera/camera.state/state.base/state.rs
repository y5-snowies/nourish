// The camera no longer has its own world-storage slot: it lives on a viewport
// `Slot` (see `compositor_y5_viewport_state_base`). The rim reaches the focused
// camera via the `VIEWPORTS` slot's active-slot accessors.

#[derive(Default)]
pub struct Camera {
    pub transform: compositor_y5_camera_transform_state::state::Transform,
    pub zone: compositor_y5_camera_zone_state::state::CameraZone,
    pub position_previous: smithay::utils::Point<f64, smithay::utils::Logical>,
    /// Momentum-pan state (touchpad two-finger swipe). `pan_accum` collects the
    /// world-space pan delta within a frame; the camera system converts it to a
    /// per-second `pan_velocity` each tick, then — once the swipe ends — coasts
    /// the camera along that velocity with exponential friction. `panning` is true
    /// while the swipe is live (velocity is measured, not yet coasting);
    /// `pan_idle_frames` counts frames with no pan delta to detect the lift-off.
    pub pan_velocity: smithay::utils::Point<f64, smithay::utils::Logical>,
    pub pan_accum: smithay::utils::Point<f64, smithay::utils::Logical>,
    pub panning: bool,
    pub pan_idle_frames: u32,
    /// Set when the touchpad reports lift-off (terminating 0,0 finger axis), so the
    /// coast launches on the NEXT tick with no idle delay — the snappy release.
    pub pan_ending: bool,
}


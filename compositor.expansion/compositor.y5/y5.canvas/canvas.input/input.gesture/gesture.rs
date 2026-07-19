use compositor_orchestration_core_state_base::Loop;
use compositor_support_action_camera_find_base::find::{Angle, Direction, Snap};

/// Number of fingers that triggers directional navigation.
const FINGERS: u32 = 3;

/// Minimum absolute scale change before a four-finger pinch counts as intentional
/// (0.2 = a 20% squeeze/spread), filtering incidental finger drift.
const PINCH_FOUR_THRESHOLD: f64 = 0.2;

/// Called from the rim on a FOUR-finger `GesturePinchEnd` with the final absolute
/// scale (relative to begin). Like pinch-to-zoom: a decisive magnify (spread, scale
/// > 1) frames a single window (focused, else most-centered); a decisive pinch
/// (squeeze, scale < 1) zooms out to an overview of all windows. Mid-range scales
/// (an indecisive pinch) do nothing.
pub fn pinch_four(loop_: &mut Loop, scale: f64) {
    if scale >= 1.0 + PINCH_FOUR_THRESHOLD {
        compositor_y5_navigator_interface_base::interface::fit_window(loop_);
    } else if scale <= 1.0 - PINCH_FOUR_THRESHOLD {
        compositor_y5_navigator_interface_base::interface::fit_all(loop_);
    }
}

/// Minimum accumulated swipe distance (libinput logical units) before a swipe
/// counts as an intentional directional flick rather than incidental motion.
const SWIPE_MAGNITUDE_THRESHOLD: f64 = 100.0;

/// Snap granularity for the swipe angle — matches the remote/gRPC view path
/// (`compositor.remote` builds `Snap::Sixteenth`) so all input sources agree.
const SNAP: Snap = Snap::Sixteenth;

/// Called from the rim on `GestureSwipeEnd`. Reads the seat's swipe accumulator
/// (latched on the Orchestrator across Begin→Update*→End) and, on an intentional
/// 3-finger swipe, drives the focused world's directional view toward the swipe
/// angle. One-shot: the navigator eases the rest.
///
/// Angle convention matches `find.angle` / the MX-gesture daemon: degrees in
/// [0, 360), 0° = Right, 90° = Down (clockwise). libinput `delta_y` is
/// screen-down-positive, so `atan2(acc_y, acc_x)` needs no sign flip.
pub fn swipe_end(loop_: &mut Loop, cancelled: bool) {
    let (fingers, mut ax, mut ay) = {
        let acc = &loop_.inner.gesture;
        (acc.fingers, acc.acc_x, acc.acc_y)
    };

    if cancelled || fingers != FINGERS {
        return;
    }

    // Natural scrolling: invert the swipe direction so a multi-finger flick moves
    // the view the same way a two-finger pan does (consistent with the axis path).
    if loop_.inner.preference.input_natural_scroll {
        ax = -ax;
        ay = -ay;
    }

    let magnitude = (ax * ax + ay * ay).sqrt();
    if magnitude < SWIPE_MAGNITUDE_THRESHOLD {
        return;
    }

    let angle = ay.atan2(ax).to_degrees().rem_euclid(360.0);
    trace!("3-finger swipe: angle={angle:.1} magnitude={magnitude:.1}");

    // `alternative = true` selects the same placement preset as Super+Left/Right
    // (view_directional's `set_4`: RAYCAST_STRETCH + ZOOM_GOAL_MIN_CHANGE), so the
    // swipe fits the target window to the screen with zoom instead of just nudging.
    compositor_y5_navigator_interface_base::interface::move_direction(
        loop_,
        Direction::Diagonal(Angle(angle), SNAP),
        true,
    );
}

//! Arrow-key navigation: the cell one step in a SCREEN direction from another.
//!
//! A sibling of `pick.base` — both answer "which cell?" about the sphere as it
//! is actually drawn, rather than reading the cube's fixed face axes, so both
//! agree with what the user is looking at. This one answers it for a key press.

use bevy::math::{Quat, Vec3};
use compositor_y5_picker_three_constant::{NAV_REACH, NAV_STEP, PITCH_MAX};
use compositor_y5_picker_three_layout::cell_at_direction;
use compositor_y5_picker_three_orient::orient::Orient;

/// The cell at the centre of the screen for `orient` — where the ray through the
/// middle of the viewport meets the sphere, which is a plain un-rotation of +Z.
fn centred(orient: Orient) -> usize {
    cell_at_direction((Quat::from_array(orient.quat()).inverse() * Vec3::Z).normalize())
}

/// The cell one step in a SCREEN direction from `cell` — `du` right, `dv` up, as
/// the sphere is oriented by `orient` (pass `target`, which centres `cell`).
/// Returns `cell` when nothing else lies that way.
///
/// Swept, not read off the cell's own face axes. Those are welded to the cube,
/// so "up" meant whichever way that face happened to be turned — a third of all
/// presses sent the selection somewhere other than the key's screen direction.
///
/// The sweep runs in the YAW/PITCH control space rather than free world space,
/// which is load-bearing: a world-space arc turns happily over a pole and finds
/// a cell the CLAMPED turntable cannot represent, so a held key reached the pole
/// and then bounced between two cells forever. Turning the controls the user
/// actually has can only reach a reachable cell — and it makes "up" at the pole
/// correctly do nothing, there being nothing above it.
///
/// The caller re-centres via `orient::face`, so the barely-past-the-boundary
/// orientation found here is a search state, never the resting one.
pub fn neighbor(cell: usize, du: i32, dv: i32, orient: Orient) -> usize {
    // Centring the cell to the RIGHT means turning the sphere left, hence the
    // negated yaw; pitch grows upward, matching `orient::drag`.
    let (dyaw, dpitch) = (-du as f32 * NAV_STEP, dv as f32 * NAV_STEP);
    let mut swept = orient;
    for _ in 0..(NAV_REACH / NAV_STEP) as i32 {
        let next = Orient {
            yaw: swept.yaw + dyaw,
            pitch: (swept.pitch + dpitch).clamp(-PITCH_MAX, PITCH_MAX),
        };
        // Unchanged = held against the pitch clamp (or no direction asked for).
        if next == swept {
            break;
        }
        swept = next;
        let hit = centred(swept);
        if hit != cell {
            return hit;
        }
    }
    cell
}

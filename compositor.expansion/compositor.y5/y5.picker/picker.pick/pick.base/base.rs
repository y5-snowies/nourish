//! Click picking: which cell the pointer is over. Casts a ray from the camera
//! through the pointer, intersects the sphere, un-rotates the hit by the
//! sphere's current rotation, and maps it to a cell. Returns None if the ray
//! misses the sphere (the pointer is outside the silhouette).
//!
//! `pick.navigate` is the sibling that answers the same question for a key.

use bevy::math::{Quat, Vec3};
use compositor_y5_picker_three_constant::{camera_distance, CAMERA_FOV_RAD, SPHERE_RADIUS};
use compositor_y5_picker_three_layout::cell_at_direction;
use compositor_y5_picker_three_orient::orient::Orient;

/// The cell under the pointer, or `None` when the ray misses the sphere (the
/// pointer is outside the silhouette).
///
/// `pointer` and `output` are in output pixels; `zoom` is the picker's camera
/// zoom — it MOVES the camera (`camera_distance`), so it changes both the
/// silhouette's screen size and where on the sphere a given pixel lands. Pass
/// the live `PickerActive::zoom`, never a constant.
pub fn pick_cell(
    pointer: (f64, f64),
    output: (f64, f64),
    orient: Orient,
    zoom: f32,
) -> Option<usize> {
    let (w, h) = output;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    // Normalized device coords (y up), camera-space ray (camera looks down -Z).
    let ndc_x = (pointer.0 / w * 2.0 - 1.0) as f32;
    let ndc_y = (1.0 - pointer.1 / h * 2.0) as f32;
    let aspect = (w / h) as f32;
    let tan = (CAMERA_FOV_RAD * 0.5).tan();
    let dir = Vec3::new(ndc_x * tan * aspect, ndc_y * tan, -1.0).normalize();
    let origin = Vec3::new(0.0, 0.0, camera_distance(zoom));

    // Ray vs sphere (center origin, radius SPHERE_RADIUS); nearest hit.
    let b = origin.dot(dir);
    let c = origin.length_squared() - SPHERE_RADIUS * SPHERE_RADIUS;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    if t < 0.0 {
        return None;
    }
    let hit = origin + dir * t;

    // Un-rotate into the sphere's local frame, then map to a cell.
    let local = Quat::from_array(orient.quat()).inverse() * hit;
    Some(cell_at_direction(local.normalize()))
}

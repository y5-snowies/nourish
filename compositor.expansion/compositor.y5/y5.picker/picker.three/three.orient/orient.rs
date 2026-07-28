//! Sphere orientation as a quaternion (xyzw), shared by the compositor and the
//! bevy scene. Drag is a view-space trackball; selecting a cell re-faces it to
//! the camera (animated via `approach`). The camera is static at +Z, so world
//! X/Y are screen right/up.

use bevy::math::{Quat, Vec3};
use compositor_y5_picker_three_constant::{CELLS_PER_FACE, REFERENCE_HZ};
use compositor_y5_picker_three_layout::{cell_at_direction, cell_grid_point, face_basis};

/// Identity orientation (no rotation).
pub const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Unit center direction of a cell, in the sphere's local frame.
pub fn cell_dir(cell: usize) -> Vec3 {
    cell_grid_point(cell, 0.5, 0.5).normalize()
}

/// The cell adjacent to `cell` stepping `du` cells along its face's right axis
/// and `dv` along its up axis (the cell's OWN left/right/up/down, independent of
/// the sphere's current rotation). A step past a face edge makes that axis
/// dominate, so `cell_at_direction` lands on the correct neighbouring face.
pub fn neighbor(cell: usize, du: i32, dv: i32) -> usize {
    let per_face = CELLS_PER_FACE * CELLS_PER_FACE;
    let cpf = CELLS_PER_FACE as f32;
    let (col, row) = ((cell % per_face % CELLS_PER_FACE) as f32, (cell % per_face / CELLS_PER_FACE) as f32);
    let (n, u, v) = face_basis((cell / per_face).min(5));
    let s = ((col + 0.5) / cpf) * 2.0 - 1.0 + du as f32 * (2.0 / cpf);
    let t = ((row + 0.5) / cpf) * 2.0 - 1.0 + dv as f32 * (2.0 / cpf);
    cell_at_direction((n + u * s + v * t).normalize())
}

/// Orientation that brings `cell` to face the camera (its outward normal → +Z).
pub fn face(cell: usize) -> [f32; 4] {
    Quat::from_rotation_arc(cell_dir(cell), Vec3::Z).to_array()
}

/// This frame's length measured in `REFERENCE_HZ` frames — the exponent that
/// converts a per-reference-frame rate into this frame's share of it.
fn frames(dt: f32) -> f32 {
    dt * REFERENCE_HZ
}

/// Scale a rotation's ANGLE by `k`, keeping its axis (`k == 1` is the identity
/// operation). Equivalent to `Quat::IDENTITY.slerp(q, k)` for `k` in 0..=1, but
/// defined for `k > 1` too, which a frame longer than the reference needs.
fn scaled(q: Quat, k: f32) -> Quat {
    let (axis, angle) = q.to_axis_angle();
    Quat::from_axis_angle(axis, angle * k)
}

/// Slerp `orientation` toward `target` by `rate` — the fraction of the remaining
/// angle covered in one `REFERENCE_HZ` frame, converted to this frame's `dt` so
/// the glide takes the same WALL TIME at any refresh rate. Snaps once close, so
/// a selection animates smoothly to face the camera then settles.
pub fn approach(orientation: [f32; 4], target: [f32; 4], rate: f32, dt: f32) -> [f32; 4] {
    let (a, b) = (Quat::from_array(orientation), Quat::from_array(target));
    if a.angle_between(b) < 1.0e-3 {
        return target;
    }
    // Remaining fraction compounds per frame, so the dt conversion is a power.
    a.slerp(b, 1.0 - (1.0 - rate).powf(frames(dt))).to_array()
}

/// View-space trackball: compose an incremental rotation (around screen up/right)
/// onto the orientation. `dx`/`dy` are in radians. Returns (new orientation, the
/// increment) — the increment seeds release momentum.
pub fn drag(orientation: [f32; 4], dx: f32, dy: f32) -> ([f32; 4], [f32; 4]) {
    let inc = Quat::from_axis_angle(Vec3::Y, dx) * Quat::from_axis_angle(Vec3::X, dy);
    let new = (inc * Quat::from_array(orientation)).normalize();
    (new.to_array(), inc.to_array())
}

/// Apply the spin to the orientation and decay it toward identity. `spin` is an
/// angular VELOCITY expressed as the rotation covered in one `REFERENCE_HZ`
/// frame, so both the step and the decay scale with `dt` — the globe coasts at
/// the same speed, for the same duration, at 60 Hz and at 240 Hz. Returns
/// (new orientation, decayed spin).
pub fn momentum(orientation: [f32; 4], spin: [f32; 4], decay: f32, dt: f32) -> ([f32; 4], [f32; 4]) {
    let (s, f) = (Quat::from_array(spin), frames(dt));
    let new_o = (scaled(s, f) * Quat::from_array(orientation)).normalize();
    let new_s = scaled(s, decay.powf(f));
    (new_o.to_array(), new_s.to_array())
}

/// Whether a spin is still meaningfully rotating (else momentum has settled).
pub fn spinning(spin: [f32; 4]) -> bool {
    Quat::from_array(spin).angle_between(Quat::IDENTITY) > 1.0e-3
}

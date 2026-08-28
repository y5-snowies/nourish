//! Cube-sphere cell placement: maps a cell index to a pose on the sphere.

use bevy::math::{Quat, Vec3};
use compositor_y5_picker_three_constant::{CELLS_PER_FACE, CELL_FILL, SPHERE_RADIUS};

/// Inverse of cell placement: which cell index a unit direction (in the
/// sphere's local frame) points at. Used by the compositor's click picking.
pub fn cell_at_direction(dir: Vec3) -> usize {
    let (ax, ay, az) = (dir.x.abs(), dir.y.abs(), dir.z.abs());
    let face = if ax >= ay && ax >= az {
        if dir.x >= 0.0 { 0 } else { 1 }
    } else if ay >= az {
        if dir.y >= 0.0 { 2 } else { 3 }
    } else if dir.z >= 0.0 {
        4
    } else {
        5
    };
    let (n, u, v) = face_basis(face);
    let dn = dir.dot(n);
    let cpf = CELLS_PER_FACE as f32;
    let s = if dn != 0.0 { dir.dot(u) / dn } else { 0.0 };
    let t = if dn != 0.0 { dir.dot(v) / dn } else { 0.0 };
    let max = CELLS_PER_FACE as i32 - 1;
    let col = (((s + 1.0) * 0.5 * cpf).floor() as i32).clamp(0, max) as usize;
    let row = (((t + 1.0) * 0.5 * cpf).floor() as i32).clamp(0, max) as usize;
    face * CELLS_PER_FACE * CELLS_PER_FACE + row * CELLS_PER_FACE + col
}

pub struct CellPose {
    pub translation: Vec3,
    pub rotation: Quat,
    /// Square edge length of the (flat) cell placeholder.
    pub edge: f32,
}

/// A point on cell `index`'s CURVED sphere patch at normalized `(su, tv)` in
/// `[0,1]` (su across, tv up). The patch covers the cell's `CELL_FILL` fraction
/// of its face quad, projected onto the sphere — so cells sit exactly on it.
pub fn cell_grid_point(index: usize, su: f32, tv: f32) -> Vec3 {
    let per_face = CELLS_PER_FACE * CELLS_PER_FACE;
    let face = (index / per_face).min(5);
    let local = index % per_face;
    let col = (local % CELLS_PER_FACE) as f32;
    let row = (local / CELLS_PER_FACE) as f32;
    let cpf = CELLS_PER_FACE as f32;
    let (n, u, v) = face_basis(face);
    // Inset the sampled fraction by CELL_FILL so cells have a gap between them.
    let pad = (1.0 - CELL_FILL) * 0.5;
    let fs = pad + su * CELL_FILL;
    let ft = pad + tv * CELL_FILL;
    let s = ((col + fs) / cpf) * 2.0 - 1.0;
    let t = ((row + ft) / cpf) * 2.0 - 1.0;
    (n + u * s + v * t).normalize() * SPHERE_RADIUS
}

/// Per-face basis: (outward normal, in-plane right, in-plane up), right-handed.
pub fn face_basis(face: usize) -> (Vec3, Vec3, Vec3) {
    match face {
        0 => (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        1 => (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        2 => (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        3 => (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        4 => (Vec3::Z, Vec3::X, Vec3::Y),
        _ => (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    }
}

/// Pose for cell `index` (0..CELL_COUNT). Cells placeholder each cube face in a
/// `CELLS_PER_FACE²` grid, projected outward onto the sphere.
pub fn cell_pose(index: usize) -> CellPose {
    let per_face = CELLS_PER_FACE * CELLS_PER_FACE;
    let face = (index / per_face).min(5);
    let local = index % per_face;
    let col = local % CELLS_PER_FACE;
    let row = local / CELLS_PER_FACE;

    let (n, u, v) = face_basis(face);
    let cpf = CELLS_PER_FACE as f32;
    // Cell center in face-local [-1, 1].
    let s = ((col as f32 + 0.5) / cpf) * 2.0 - 1.0;
    let t = ((row as f32 + 0.5) / cpf) * 2.0 - 1.0;

    // Cube-face point, then projected to the sphere.
    let dir = (n + u * s + v * t).normalize();
    let translation = dir * SPHERE_RADIUS;
    // Orient the cell's front face (+Z) outward along the sphere normal.
    let rotation = Quat::from_rotation_arc(Vec3::Z, dir);
    let edge = (2.0 / cpf) * CELL_FILL;

    CellPose { translation, rotation, edge }
}

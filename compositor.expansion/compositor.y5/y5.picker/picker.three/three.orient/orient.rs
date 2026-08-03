//! Sphere orientation as YAW + PITCH, shared by the compositor and the bevy
//! scene. The camera is static at +Z, so world X/Y are screen right/up.
//!
//! NOT a free quaternion, and that is the point: a quaternion trackball
//! accumulates ROLL, and `from_rotation_arc(cell, +Z)` — the shortest arc —
//! carries no up-vector at all, so a lock-on twisted the globe by whatever roll
//! that arc held, which varied with the cell. Yaw/pitch has no roll to gather.

use bevy::math::{Quat, Vec3};
use compositor_y5_picker_three_constant::{PITCH_MAX, REFERENCE_HZ};
use compositor_y5_picker_three_layout::cell_grid_point;

/// Yaw about the sphere's upright axis, pitch about the screen's horizontal one
/// (radians). Doubles as momentum, where both are per-`REFERENCE_HZ`-frame rates.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Orient { pub yaw: f32, pub pitch: f32 }

impl Orient {
    /// Facing the +Z pole, upright; as a velocity, "not spinning".
    pub const ZERO: Self = Self { yaw: 0.0, pitch: 0.0 };

    /// The rotation the scene applies (xyzw). Pitch OUTER, yaw inner — the order
    /// that keeps a horizontal drag spinning the globe about its own axis.
    pub fn quat(self) -> [f32; 4] {
        (Quat::from_rotation_x(self.pitch) * Quat::from_rotation_y(self.yaw)).to_array()
    }
}

/// Unit center direction of a cell, in the sphere's local frame.
pub fn cell_dir(cell: usize) -> Vec3 {
    cell_grid_point(cell, 0.5, 0.5).normalize()
}

/// The upright orientation bringing `cell` to the camera. `yaw_now` is KEPT for a
/// polar cell, where yaw only spins the pole in place and 0 would swing it round.
pub fn face(cell: usize, yaw_now: f32) -> Orient {
    let d = cell_dir(cell);
    let flat = d.x.hypot(d.z);
    let yaw = if flat < 1.0e-4 { yaw_now } else { (-d.x).atan2(d.z) };
    Orient { yaw, pitch: d.y.atan2(flat) }
}

/// This frame's length in `REFERENCE_HZ` frames — a rate's share of this frame.
fn frames(dt: f32) -> f32 { dt * REFERENCE_HZ }

/// Shortest signed yaw delta, wrapped into (-PI, PI]: never the long way round.
fn short(delta: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (delta + PI).rem_euclid(TAU) - PI
}

/// Turntable drag: yaw follows horizontal motion, pitch vertical, and pitch alone
/// is clamped (`PITCH_MAX`) so the globe never tumbles over a pole. Returns the
/// increment ACTUALLY applied — a drag held at the clamp must not seed momentum.
pub fn drag(o: Orient, dx: f32, dy: f32) -> (Orient, Orient) {
    let next = Orient { yaw: o.yaw + dx, pitch: (o.pitch + dy).clamp(-PITCH_MAX, PITCH_MAX) };
    (next, Orient { yaw: dx, pitch: next.pitch - o.pitch })
}

/// Ease toward `target` by `rate` — the fraction of the REMAINING angle covered
/// in one `REFERENCE_HZ` frame, converted to this `dt` so the glide takes the
/// same wall time at any refresh rate. Snaps once close.
pub fn approach(o: Orient, target: Orient, rate: f32, dt: f32) -> Orient {
    let (dyaw, dpitch) = (short(target.yaw - o.yaw), target.pitch - o.pitch);
    if dyaw.abs() < 1.0e-3 && dpitch.abs() < 1.0e-3 { return target; }
    let k = 1.0 - (1.0 - rate).powf(frames(dt));
    Orient { yaw: o.yaw + dyaw * k, pitch: o.pitch + dpitch * k }
}

/// Play out drag-release momentum and decay it. `spin` is per `REFERENCE_HZ`
/// frame, so step and decay both scale with `dt` — the globe coasts at the same
/// speed, for the same duration, at 60 Hz and at 240 Hz.
pub fn momentum(o: Orient, spin: Orient, decay: f32, dt: f32) -> (Orient, Orient) {
    let f = frames(dt);
    let pitch = (o.pitch + spin.pitch * f).clamp(-PITCH_MAX, PITCH_MAX);
    let d = decay.powf(f);
    (Orient { yaw: o.yaw + spin.yaw * f, pitch }, Orient { yaw: spin.yaw * d, pitch: spin.pitch * d })
}

/// Whether a spin is still meaningfully rotating (else momentum has settled).
pub fn spinning(spin: Orient) -> bool { spin.yaw.abs() + spin.pitch.abs() > 1.0e-3 }

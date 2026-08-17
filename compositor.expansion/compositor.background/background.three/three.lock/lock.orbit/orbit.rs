use bevy::camera::Camera3d;
use bevy::prelude::*;
use compositor_background_three_lock_constant::{CAMERA_DISTANCE, CAMERA_ORBIT_SPEED};
use compositor_background_three_lock_state::{MorphAnim, MorphPhase};

pub fn apply_camera_orbit(
    anim: Res<MorphAnim>,
    time: Res<Time>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut state: Local<f32>,
) {
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };

    let elevation_target = match anim.phase {
        MorphPhase::Idle | MorphPhase::PreMorphDelay => 0.0,
        _ => 0.0,
    };

    let should_orbit = !matches!(anim.phase, MorphPhase::Idle | MorphPhase::PreMorphDelay);
    if should_orbit {
        *state += CAMERA_ORBIT_SPEED * time.delta_secs();
    } else {
        *state = 0.0;
    }

    let angle = *state;
    let pos = Vec3::new(
        CAMERA_DISTANCE * angle.sin(),
        elevation_target,
        CAMERA_DISTANCE * angle.cos(),
    );
    *transform = Transform::from_translation(pos).looking_at(Vec3::ZERO, Vec3::Y);
}

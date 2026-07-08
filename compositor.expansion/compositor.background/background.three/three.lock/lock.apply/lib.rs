//! Apply the animation state to the material params and mesh transform.

use bevy::prelude::*;
use compositor_background_three_lock_constant::{
    CAMERA_ORBIT_SPEED, HERO_POSITION, HERO_SCALE, SPIN_SPEED,
};
use compositor_background_three_lock_material::MorphMaterial;
use compositor_background_three_lock_state::{MorphAnim, MorphPhase, MorphPlane};

pub fn apply_to_material(
    anim: Res<MorphAnim>,
    mut materials: ResMut<Assets<MorphMaterial>>,
    query: Query<&MeshMaterial3d<MorphMaterial>, With<MorphPlane>>,
) {
    for handle in query.iter() {
        if let Some(mut mat) = materials.get_mut(&handle.0) {
            mat.params.t = anim.t;
            mat.params.going_to_sphere = anim.going_to_sphere;
            mat.params.ready = anim.render_ready;
        }
    }
}

pub fn apply_to_transform(
    anim: Res<MorphAnim>,
    time: Res<Time>,
    mut query: Query<&mut Transform, With<MorphPlane>>,
) {
    let Ok(mut transform) = query.single_mut() else {
        return;
    };

    let pos = Vec3::ZERO.lerp(HERO_POSITION, anim.hero);
    let scale = 1.0 + (HERO_SCALE - 1.0) * anim.hero;

    // Object spin only during sphere phases (full sphere or hero).
    // Suppressed during Morphing/Unmorphing — those are visually busy
    // enough without rotation. Also suppressed when camera orbit is on
    // (orbit shows depth from another angle; spinning would compound).
    let is_spinning = CAMERA_ORBIT_SPEED == 0.0
        && matches!(
            anim.phase,
            MorphPhase::SphereFull
                | MorphPhase::ShrinkingToHero
                | MorphPhase::Hero
                | MorphPhase::GrowingFromHero
                | MorphPhase::SphereFullReverse
        );
    let spin_increment = if is_spinning {
        SPIN_SPEED * time.delta_secs()
    } else {
        0.0
    };

    let new_rotation = transform.rotation * Quat::from_axis_angle(Vec3::Y, spin_increment);
    *transform = Transform {
        translation: pos,
        rotation: new_rotation,
        scale: Vec3::splat(scale),
    };
}

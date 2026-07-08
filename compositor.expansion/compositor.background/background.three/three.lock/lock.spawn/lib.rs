//! Startup system: camera + morph mesh + material.

use bevy::camera::{Camera, Camera3d, ClearColorConfig, ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use compositor_background_three_lock_constant::{
    AMBIENT_INTENSITY, CAMERA_DISTANCE, CAMERA_ELEVATION, CAMERA_FOV_RAD, LIGHT_DIR,
    LIGHT_INTENSITY, MESH_RESOLUTION, SPHERE_RADIUS,
};
use compositor_background_three_lock_material::{MorphMaterial, MorphParams};
use compositor_background_three_lock_state::{MorphAnim, MorphConfig, MorphPlane};

pub fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MorphMaterial>>,
    mut anim: ResMut<MorphAnim>,
    config: Res<MorphConfig>,
) {
    // Make the visible startup state a flat plane (flatness = 1.0).
    // This matches the spec: "first frame is the screen as a flat plane;
    // when locked, it folds into a sphere."
    anim.t = 1.0;
    anim.going_to_sphere = 0.0;

    commands.spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: CAMERA_FOV_RAD,
            aspect_ratio: config.output_aspect,
            near: 0.1,
            far: 1000.0,
            ..default()
        }),
        Transform::from_xyz(0.0, CAMERA_ELEVATION, CAMERA_DISTANCE).looking_at(Vec3::ZERO, Vec3::Y),
        RenderTarget::Image(ImageRenderTarget::from(config.output_handle.clone())),
    ));

    let mesh_handle = meshes.add(compositor_background_three_lock_mesh::build_morph_mesh(
        MESH_RESOLUTION,
        MESH_RESOLUTION,
        SPHERE_RADIUS,
        config.output_aspect, // ← from MorphConfig
    ));
    let material_handle = materials.add(MorphMaterial {
        params: MorphParams {
            t: 1.0,
            going_to_sphere: 0.0,
            plane_aspect: config.output_aspect,
            sphere_radius: SPHERE_RADIUS,
            light_dir_x: LIGHT_DIR.x,
            light_dir_y: LIGHT_DIR.y,
            light_dir_z: LIGHT_DIR.z,
            light_intensity: LIGHT_INTENSITY,
            ambient_intensity: AMBIENT_INTENSITY,
            ready: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        },
        snapshot: config.snapshot_handle.clone(),
    });

    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform::IDENTITY,
        MorphPlane,
    ));
}

//! Per-frame systems: static camera, sphere orientation, and selection visuals.

use bevy::prelude::*;
use compositor_y5_picker_three_constant::camera_distance;
use compositor_y5_picker_three_layout::cell_pose;
use compositor_y5_picker_three_mesh::cell_border_points;
use compositor_y5_picker_three_state::{
    PickerCamera, PickerOutline, PickerOutlineMesh, PickerPlus, PickerRoot,
    PickerSelected, PickerTransform,
};

/// Force each cell material's bind group to rebuild every frame so it picks up
/// the world thumbnail the bridge swaps into the placeholder `GpuImage` in place
/// (an in-place swap doesn't invalidate an already-built bind group). The
/// deref-mut is load-bearing: `Assets::get_mut` only emits `AssetEvent::Modified`
/// — which is what triggers the rebuild — when the returned guard is actually
/// written through; a bare `get_mut` marks nothing. Mirrors the lock screen's
/// `apply_to_material`.
pub fn refresh_cell_materials(
    cells: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for handle in cells.iter() {
        if let Some(mut m) = materials.get_mut(&handle.0) {
            let _ = &mut *m; // deref-mut → Modified → bind-group rebuild
        }
    }
}

/// Static camera on +Z, looking at the origin. Distance comes from the zoom; the
/// sphere (not the camera) carries all rotation, so screen axes stay world X/Y,
/// which keeps the view-space trackball + click-picking aligned.
pub fn idle_camera(
    transform: Res<PickerTransform>,
    mut cam: Query<&mut Transform, With<PickerCamera>>,
) {
    let distance = camera_distance(transform.zoom);
    for mut tf in &mut cam {
        tf.translation = Vec3::new(0.0, 0.0, distance);
        tf.look_at(Vec3::ZERO, Vec3::Y);
    }
}

/// Apply the sphere's orientation quaternion (compositor-authoritative).
pub fn apply_rotation(transform: Res<PickerTransform>, mut root: Query<&mut Transform, With<PickerRoot>>) {
    let q = Quat::from_array(transform.orientation);
    for mut tf in &mut root {
        tf.rotation = q;
    }
}

/// Rebuild the outline to the focused cell's curved border (on change) and place
/// the "+" on it, toggling their visibility with the selection.
pub fn apply_selection(
    selected: Res<PickerSelected>,
    outline_mesh: Option<Res<PickerOutlineMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut outline_vis: Query<&mut Visibility, (With<PickerOutline>, Without<PickerPlus>)>,
    mut plus: Query<(&mut Transform, &mut Visibility), (With<PickerPlus>, Without<PickerOutline>)>,
) {
    if selected.is_changed()
        && let Some(sel) = selected.0
        && let Some(outline_mesh) = outline_mesh.as_ref()
        && let Some(mut mesh) = meshes.get_mut(&outline_mesh.0)
    {
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, cell_border_points(sel));
    }

    let visible = selected.0.is_some();
    for mut vis in &mut outline_vis {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }

    let pose = selected.0.map(cell_pose);
    for (mut tf, mut vis) in &mut plus {
        match &pose {
            Some(p) => {
                // Nudge the "+" in front of the tile so it isn't z-fought.
                tf.translation = p.translation + p.rotation * (Vec3::Z * (p.edge * 0.06));
                tf.rotation = p.rotation;
                tf.scale = Vec3::splat(p.edge);
                *vis = Visibility::Visible;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

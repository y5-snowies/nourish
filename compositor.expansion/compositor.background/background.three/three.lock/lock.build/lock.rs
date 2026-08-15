use bevy::prelude::*;
use compositor_support_bevy_core_bridge_base::{BridgeDirection, BridgeEntry, BridgeRegistry};
use compositor_support_bevy_core_placeholder_base::create_input_placeholder;
use compositor_background_three_lock_material::MorphMaterial;
use compositor_background_three_lock_shader::MorphScenePlugin;
use compositor_background_three_lock_state::{
    MorphAnim, MorphCommand, MorphConfig, MorphPhase, SNAPSHOT_LABEL,
};

use compositor_model_debug_instance_record::warn;
use bevy::pbr::MaterialPlugin;
use std::sync::{Arc, Mutex};

pub fn build(
    snapshot_size: (u32, u32),
    output_aspect: f32,
    snapshot_wgpu_tex: Arc<wgpu::Texture>,
    app: &mut App,
    output: Handle<Image>,
) {
    let snapshot_handle = {
        let world = app.world_mut();
        let mut images = world.resource_mut::<Assets<Image>>();
        create_input_placeholder(&mut images, snapshot_size)
    };
    {
        let registry = app.world().resource::<BridgeRegistry>().clone();
        registry.push(BridgeEntry {
            texture: snapshot_wgpu_tex,
            handle: snapshot_handle.clone(),
            installed: Arc::new(Mutex::new(false)),
            direction: BridgeDirection::Input,
            label: SNAPSHOT_LABEL,
        });
    }

    app.add_plugins(MorphScenePlugin)
        .add_plugins(MaterialPlugin::<MorphMaterial>::default())
        .insert_resource(MorphConfig {
            snapshot_handle: snapshot_handle.clone(),
            output_handle: output.clone(),
            output_aspect,
        })
        .insert_resource(MorphAnim::default())
        .add_systems(Startup, compositor_background_three_lock_spawn::spawn)
        .add_systems(
            Update,
            (
                compositor_background_three_lock_anim::tick_animation,
                compositor_background_three_lock_apply::apply_to_material,
                compositor_background_three_lock_apply::apply_to_transform,
                compositor_background_three_lock_orbit::apply_camera_orbit,
            )
                .chain(),
        );
}

pub fn apply_command(world: &mut World, command: MorphCommand) {
    let elapsed = world
        .get_resource::<Time>()
        .map(|t| t.elapsed_secs_f64())
        .unwrap_or(0.0);

    match command {
        MorphCommand::Lock => {
            let mut anim = world.resource_mut::<MorphAnim>();
            anim.phase = MorphPhase::PreMorphDelay;
            anim.phase_started_at = elapsed;
            anim.t = 0.0;
            anim.going_to_sphere = 1.0;
            anim.hero = 0.0;
        }
        MorphCommand::Unlock => {
            let mut anim = world.resource_mut::<MorphAnim>();
            anim.phase = MorphPhase::GrowingFromHero;
            anim.phase_started_at = elapsed;
        }
        MorphCommand::SetPhase(p) => {
            let mut anim = world.resource_mut::<MorphAnim>();
            anim.phase = p;
            anim.phase_started_at = elapsed;
        }
        MorphCommand::SetSnapshot(new_tex) => {
            let registry = world.resource::<BridgeRegistry>().clone();
            if !registry.replace_texture(SNAPSHOT_LABEL, new_tex) {
                warn!(
                    "MorphScene::apply_command(SetSnapshot): no bridge entry \
                 labelled '{}' found",
                    SNAPSHOT_LABEL,
                );
            }
        }
    }
}

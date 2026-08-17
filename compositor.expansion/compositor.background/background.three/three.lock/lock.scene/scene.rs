use bevy::asset::Handle;
use bevy::image::Image;
use bevy::prelude::{App, World};
use std::sync::Arc;
pub use compositor_background_three_lock_material::MorphMaterial;
pub use compositor_background_three_lock_shader::MorphScenePlugin;
pub use compositor_background_three_lock_state::{MorphCommand, MorphPhase};

pub struct MorphScene {
    pub snapshot_size: (u32, u32),
    pub output_aspect: f32,
    pub snapshot_wgpu_tex: Arc<wgpu::Texture>,
}

impl MorphScene {
    pub fn new(snapshot_size: (u32, u32), snapshot_wgpu_tex: Arc<wgpu::Texture>) -> Self {
        let output_aspect = snapshot_size.0 as f32 / snapshot_size.1 as f32;
        Self {
            snapshot_size,
            output_aspect,
            snapshot_wgpu_tex,
        }
    }
}

impl compositor_support_bevy_core_scene_base::BevyScene for MorphScene {
    type Command = MorphCommand;

    fn build(&self, app: &mut App, output: Handle<Image>) {
        compositor_background_three_lock_build::build(
            self.snapshot_size,
            self.output_aspect,
            self.snapshot_wgpu_tex.clone(),
            app,
            output,
        )
    }

    fn apply_command(&self, world: &mut World, command: Self::Command) {
        compositor_background_three_lock_build::apply_command(world, command)
    }
}

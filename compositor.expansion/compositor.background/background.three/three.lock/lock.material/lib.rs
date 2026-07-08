use bevy::asset::{Asset, Handle};
use bevy::image::Image;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::mesh::{Mesh, MeshVertexBufferLayoutRef};
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

#[derive(ShaderType, Clone, Debug)]
pub struct MorphParams {
    /// Current animation progress in [0, 1]. Driven by the spring solver
    /// from Rust over a phase's duration.
    pub t: f32,

    /// 1.0 if we're morphing toward sphere (Lock direction).
    /// 0.0 if we're unmorphing toward plane (Unlock direction).
    /// Stays at the value of the most-recently-active phase while at rest.
    pub going_to_sphere: f32,

    pub plane_aspect: f32,
    pub sphere_radius: f32,

    pub light_dir_x: f32,
    pub light_dir_y: f32,
    pub light_dir_z: f32,

    pub light_intensity: f32,
    pub ambient_intensity: f32,

    /// 0.0 → plane fully transparent (cast not yet bound / uniform not settled),
    /// 1.0 → draw normally. Gates out the first-frame sphere/blank flash.
    pub ready: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct MorphMaterial {
    #[uniform(0)]
    pub params: MorphParams,

    #[texture(1)]
    #[sampler(2)]
    pub snapshot: Handle<Image>,
}

impl Material for MorphMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://compositor_background_three_lock_shader/morph.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://compositor_background_three_lock_shader/morph.wgsl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
let vertex_layout = layout.0.get_layout(&[
    Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
    Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
    Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
    Mesh::ATTRIBUTE_UV_1.at_shader_location(3),
])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        // No back-face culling: with the morph deforming geometry, edge
        // cases can show interior faces. With outward-facing normals
        // throughout the morph (sphere normals stay valid even when
        // partially flattened), we could enable culling, but leaving it
        // off is safer.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

use std::f32::consts::PI;
use bevy::asset::RenderAssetUsages;
use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};

/// Build a grid-topology sphere mesh.
///
/// `resolution` controls how many subdivisions in each axis. Total
/// vertices = (resolution + 1)². At 256 that's 66,049 vertices,
/// 131,072 triangles — smooth without being excessive.
pub fn build_morph_mesh(
    longitude_segments: u32,
    latitude_segments: u32,
    radius: f32,
    plane_aspect: f32,
) -> Mesh {
    // Standard UV sphere mesh with explicit plane positions stored as
    // a second attribute (via the UV channel, repurposed). Each vertex has:
    //   POSITION = sphere position (3D)
    //   NORMAL   = outward sphere normal (3D)
    //   UV_0     = plane position (2D, used as the "flattened" target)
    //   UV_1     = texture sampling coords (lng_norm, lat_norm) for the shader

    let lng_seg = longitude_segments as usize;
    let lat_seg = latitude_segments as usize;
    let vert_count = (lng_seg + 1) * (lat_seg + 1);

    let mut positions = Vec::with_capacity(vert_count);
    let mut normals = Vec::with_capacity(vert_count);
    let mut plane_positions = Vec::with_capacity(vert_count); // stored in UV_0
    let mut tex_uvs = Vec::with_capacity(vert_count); // stored in UV_1
    let mut indices = Vec::with_capacity(lng_seg * lat_seg * 6);

    for lat_i in 0..=lat_seg {
        let lat_t = lat_i as f32 / lat_seg as f32;
        let theta = lat_t * std::f32::consts::PI; // 0 (north) to π (south)
        let sin_theta = theta.sin();
        let cos_theta = theta.cos();

        for lng_i in 0..=lng_seg {
            let lng_t = lng_i as f32 / lng_seg as f32;
            let phi = (lng_t - 0.5) * 2.0 * std::f32::consts::PI; // -π to π

            // Sphere position: front (+Z) is phi=0
            let x = radius * sin_theta * phi.sin();
            let y = radius * cos_theta;
            let z = radius * sin_theta * phi.cos();
            positions.push([x, y, z]);
            normals.push([x / radius, y / radius, z / radius]);

            // Plane position: equirectangular projection scaled to screen aspect.
            // Plane is plane_aspect wide and 1 tall, centered at origin.
            let plane_x = (lng_t - 0.5) * plane_aspect;
            let plane_y = (0.5 - lat_t); // top row = +0.5, bottom = -0.5
            plane_positions.push([plane_x, plane_y]);

            // Texture UV: same as lng/lat for sampling the snapshot
            tex_uvs.push([lng_t, lat_t]);
        }
    }

    let stride = (lng_seg + 1) as u32;
    for lat_i in 0..lat_seg as u32 {
        for lng_i in 0..lng_seg as u32 {
            let i = lat_i * stride + lng_i;
            let i_right = i + 1;
            let i_down = i + stride;
            let i_down_right = i_down + 1;
            indices.extend_from_slice(&[i, i_down, i_down_right, i, i_down_right, i_right]);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    // UV_0 carries the plane position (2D)
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, plane_positions);
    // UV_1 carries the texture sampling coords
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, tex_uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

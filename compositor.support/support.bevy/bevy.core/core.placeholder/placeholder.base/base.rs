use bevy::asset::{Assets, Handle, RenderAssetUsages};
use bevy::image::Image;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};

pub fn create_output_placeholder(images: &mut Assets<Image>, size: (u32, u32)) -> Handle<Image> {
    let extent = Extent3d {
        width: size.0,
        height: size.1,
        depth_or_array_layers: 1,
    };
    let mut image = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("output_placeholder"),
            size: extent,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8UnormSrgb,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        asset_usage: RenderAssetUsages::all(),
        ..Default::default()
    };
    image.resize(extent);
    images.add(image)
}

pub fn create_input_placeholder(images: &mut Assets<Image>, size: (u32, u32)) -> Handle<Image> {
    let extent = Extent3d {
        width: size.0,
        height: size.1,
        depth_or_array_layers: 1,
    };
    let mut image = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("input_placeholder"),
            size: extent,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8UnormSrgb,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        asset_usage: RenderAssetUsages::all(),
        ..Default::default()
    };
    image.resize(extent);
    images.add(image)
}

//! `BridgeRegistryPlugin`: render-world systems that install pending
//! bridge entries (swap placeholder `GpuImage`s for dmabuf-backed ones).

use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderSystems, render_asset::RenderAssets, renderer::RenderDevice,
    texture::GpuImage,
};
use compositor_support_bevy_core_bridge_base::{BridgeDirection, BridgeRegistry};
use compositor_model_debug_instance_record::info;

pub struct BridgeRegistryPlugin;

impl Plugin for BridgeRegistryPlugin {
    fn build(&self, app: &mut App) {
        let registry = BridgeRegistry::default();
        app.insert_resource(registry.clone());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
        render_app.insert_resource(registry).add_systems(
            Render,
            install_bridges
                .in_set(RenderSystems::Prepare)
                .run_if(any_pending),
        );
    }
}

fn any_pending(registry: Res<BridgeRegistry>) -> bool {
    let Ok(entries) = registry.entries.lock() else { return false };
    entries
        .iter()
        .any(|e| e.installed.lock().ok().map(|g| !*g).unwrap_or(false))
}

fn install_bridges(
    registry: Res<BridgeRegistry>,
    mut gpu_images: ResMut<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
) {
    let Ok(entries) = registry.entries.lock() else { return };

    for entry in entries.iter() {
        let Ok(mut installed) = entry.installed.lock() else { continue };
        if *installed {
            continue;
        }
        let Some(existing) = gpu_images.get_mut(&entry.handle) else {
            // Placeholder GpuImage not prepared yet — retry next frame. If this
            // persists, the captured texture never appears (the bridge can't
            // swap it in). Diagnostic for the lock/picker capture-not-visible.
            compositor_model_debug_instance_record::warn!(
                "bridge: GpuImage not prepared for {} ({:?}); deferring",
                entry.label,
                entry.direction
            );
            continue;
        };

        let bevy_texture: bevy::render::render_resource::Texture =
            (*entry.texture).clone().into();
        let texture_view = bevy_texture.create_view(&Default::default());

        let sampler = match entry.direction {
            BridgeDirection::Output => render_device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("bridge_output_sampler"),
                ..Default::default()
            }),
            BridgeDirection::Input => render_device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("bridge_input_sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
        };

        existing.texture = bevy_texture;
        existing.texture_view = texture_view;
        existing.sampler = sampler;
        existing.texture_descriptor = wgpu::TextureDescriptor {
            label: Some(entry.label),
            size: entry.texture.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: entry.texture.dimension(),
            format: entry.texture.format(),
            usage: entry.texture.usage(),
            view_formats: &[],
        };

        *installed = true;
        info!("bridge installed ({:?}, {})", entry.direction, entry.label);
    }
}

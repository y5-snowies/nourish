//! Builds the per-instance Bevy `App` wired to our manual wgpu resources.

use std::sync::{Arc, Mutex};

use bevy::DefaultPlugins;
use bevy::prelude::*;
use bevy::render::{
    RenderDebugFlags, RenderPlugin,
    renderer::{
        RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
    },
    settings::{RenderCreation, RenderResources},
};
use compositor_support_bevy_core_bridge_base::{
    BridgeDirection, BridgeEntry, BridgeRegistry, OUTPUT_LABEL,
};
use compositor_support_bevy_core_install_base::BridgeRegistryPlugin;
use compositor_support_bevy_core_placeholder_base::create_output_placeholder;
use compositor_support_bevy_core_scene_base::BevyScene;
use compositor_support_bevy_core_shared_base::SharedContext;

/// Construct the Bevy `App` for one scene instance: manual render plugin,
/// bridge registry plugin, output placeholder + output bridge entry, then
/// `scene.build(...)`, `finish()` and `cleanup()`. Returns the app together
/// with the output image handle (the camera render target).
pub fn build_app<S: BevyScene>(
    scene: &S,
    ctx: &SharedContext,
    output_wgpu_texture: Arc<wgpu::Texture>,
    size_px: (u32, u32),
) -> (App, Handle<Image>) {
    let mut app = App::new();
    let render_plugin = build_manual_render_plugin(ctx);
    app.add_plugins(DefaultPlugins.build().set(render_plugin));
    app.add_plugins(BridgeRegistryPlugin);

    let output_handle = {
        let world = app.world_mut();
        let mut images = world.resource_mut::<Assets<Image>>();
        create_output_placeholder(&mut images, size_px)
    };

    {
        let registry = app.world().resource::<BridgeRegistry>().clone();
        registry.push(BridgeEntry {
            texture: output_wgpu_texture,
            handle: output_handle.clone(),
            installed: Arc::new(Mutex::new(false)),
            direction: BridgeDirection::Output,
            label: OUTPUT_LABEL,
        });
    }

    scene.build(&mut app, output_handle.clone());

    app.finish();
    app.cleanup();

    (app, output_handle)
}

fn build_manual_render_plugin(ctx: &SharedContext) -> RenderPlugin {
    let render_instance = RenderInstance(Arc::new(WgpuWrapper::new(ctx.instance.clone())));
    let render_adapter_info = RenderAdapterInfo(WgpuWrapper::new(ctx.adapter.get_info()));
    let render_adapter = RenderAdapter(Arc::new(WgpuWrapper::new((*ctx.adapter).clone())));
    let render_device = RenderDevice::from((*ctx.device).clone());
    let render_queue = RenderQueue(Arc::new(WgpuWrapper::new((*ctx.queue).clone())));

    RenderPlugin {
        debug_flags: RenderDebugFlags::all(),
        render_creation: RenderCreation::Manual(RenderResources(
            render_device,
            render_queue,
            render_adapter_info,
            render_adapter,
            render_instance,
        )),
        synchronous_pipeline_compilation: false,
    }
}

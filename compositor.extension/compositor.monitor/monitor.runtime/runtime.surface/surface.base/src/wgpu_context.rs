//! WGPU Vulkan context with dmabuf-import features enabled.
//!
//! This is the entry point for the WGPU side of the import stack. Create one
//! per process for the Iced subsystem (Bevy keeps its own — they don't share).

use std::sync::Arc;

use wgpu::{
    Adapter, Device, DeviceDescriptor, ExperimentalFeatures, Features, Instance, Queue,
    RequestAdapterOptions,
};

use crate::error::WgpuContextError;

/// Holds the WGPU instance/adapter/device/queue created with dmabuf-import
/// capability.
///
/// Wrapped in `Arc` so it can be shared cheaply across many `IcedSurface`
/// imports and across threads. Keep this alive for the entire program — when
/// the last `Arc` drops, all imported textures become invalid.
pub struct WgpuVulkanContext {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
    /// The `(fourcc × modifier)` set this device can import (bridge intersection
    /// input). Multi-plane modifiers survive only under `gpu_allow_dcc`.
    pub importable: smithay::backend::allocator::format::FormatSet,
}

impl WgpuVulkanContext {
    /// Wrap in `Arc` for sharing. Use this when handing the context to
    /// multiple subsystems (each `IcedSurface` keeps an `Arc<WgpuVulkanContext>`
    /// implicitly through the surface registry's shared engine).
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }
}


pub fn debug_self_test(wgpu_ctx: &crate::wgpu_context::WgpuVulkanContext) {
    info!("debug_self_test: start");

    // Allocate a normal wgpu-owned texture (no dmabuf).
    let tex = wgpu_ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("self_test_tex"),
        size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());

    let mut encoder = wgpu_ctx.device.create_command_encoder(
        &wgpu::CommandEncoderDescriptor { label: Some("self_test_encoder") }
    );
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("self_test_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    let cmd = encoder.finish();
    let idx = wgpu_ctx.queue.submit(std::iter::once(cmd));
    info!("debug_self_test: submitted, waiting...");

    let result = wgpu_ctx.device.poll(wgpu::PollType::Wait {
        timeout: None,
        submission_index: Some(idx),
    });
    info!("debug_self_test: poll returned result={result:?}");
}

/// Create a WGPU instance + Vulkan device that supports dmabuf import.
///
/// Synchronous via `pollster`. Run this from a worker thread if you want to
/// keep the compositor's main loop responsive during init — see the Bevy
/// integration's `bevy_wgpu_context_init` pattern.
pub fn create_wgpu_vulkan_context() -> Result<WgpuVulkanContext, WgpuContextError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::empty(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });
    info!("Created wgpu::Instance (Vulkan backend)");

    use compositor_developer_environment_experimental_base::base as experimental;
    // Pin the wgpu adapter to the render node by default; opt out via gpu_no_pin_wgpu_node.
    let pinned = (!experimental::get().contains(experimental::GpuFlags::NO_PIN_WGPU_NODE)).then(|| {
        let node = compositor_developer_environment_config_base::base::get().render_node.clone();
        compositor_kernel_graphic_bridge_negotiate_wgpu::query::pick_adapter(&instance, &node)
    });
    let adapter = match pinned.flatten() {
        Some(a) => {
            info!("pinned wgpu adapter to render node");
            a
        }
        None => pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|_| WgpuContextError::NoAdapter)?,
    };

    let info = adapter.get_info();
    info!(
        "Got Vulkan adapter: {} ({:?}, backend={:?})",
        info.name, info.device_type, info.backend
    );

    // We require the two dmabuf-import features. Everything else is optional
    // (we'd love texture compression, but won't fail if absent).
    let required = Features::VULKAN_EXTERNAL_MEMORY_FD | Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
    let optional = Features::TEXTURE_COMPRESSION_BC
        | Features::TEXTURE_COMPRESSION_ETC2
        | Features::TEXTURE_COMPRESSION_ASTC;

    let available = adapter.features();
    let missing_required = required - available;
    if !missing_required.is_empty() {
        return Err(WgpuContextError::MissingFeatures {
            required: missing_required,
            supported: available,
        });
    }
    let to_request = (required | optional) & available;

    let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
        experimental_features: ExperimentalFeatures::disabled(),
        label: Some("y5_iced_dmabuf_wgpu_device"),
        required_features: to_request,
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .map_err(WgpuContextError::DeviceCreation)?;

    info!("Created Vulkan Device + Queue with dmabuf import features");

    // Enumerate importable dmabuf modifiers via raw ash (wgpu has no such query).
    let importable = compositor_kernel_graphic_bridge_negotiate_wgpu::query::query_importable(
        &instance,
        &adapter,
        experimental::get().contains(experimental::GpuFlags::ALLOW_DCC),
        experimental::get().contains(experimental::GpuFlags::PROBE_MODIFIERS),
    );
    info!("iced wgpu importable dmabuf formats: {}", importable.iter().count());

    Ok(WgpuVulkanContext {
        instance,
        adapter,
        device,
        queue,
        importable,
    })
}

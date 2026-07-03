//! WGPU Vulkan context with dmabuf-import features enabled. One per process for
//! the Bevy subsystem (Iced keeps its own — they don't share).

use std::sync::Arc;

use wgpu::{
    Adapter, Device, DeviceDescriptor, ExperimentalFeatures, Features, Instance, Queue,
    RequestAdapterOptions,
};

use compositor_support_bevy_core_fault_base::WgpuContextError;
use compositor_developer_debug_instance_record::info;
use compositor_developer_environment_experimental_base::base as experimental;

pub struct WgpuVulkanContext {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
    /// The `(fourcc × modifier)` set this device can import (bridge intersection input).
    pub importable: smithay::backend::allocator::format::FormatSet,
}

impl WgpuVulkanContext {
    /// Wrap in `Arc` for sharing across `BevySurface` allocations and threads.
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }
}

/// Create a wgpu instance + Vulkan device that supports dmabuf import.
///
/// Synchronous via `pollster`. Run from a worker thread if you want to keep
/// the main loop responsive during init.
pub fn create_wgpu_vulkan_context() -> Result<WgpuVulkanContext, WgpuContextError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::empty(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });
    info!("Created wgpu::Instance (Vulkan backend)");

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
        label: Some("y5_bevy_dmabuf_wgpu_device"),
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

    Ok(WgpuVulkanContext {
        instance,
        adapter,
        device,
        queue,
        importable,
    })
}

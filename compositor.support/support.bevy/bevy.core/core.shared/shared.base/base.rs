use std::sync::Arc;

/// Process-wide wgpu resources. Bevy doesn't share `Renderer` across
/// instances; this is just the wgpu context.
#[derive(Clone)]
pub struct SharedContext {
    pub instance: wgpu::Instance,
    pub adapter: Arc<wgpu::Adapter>,
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
}

impl std::fmt::Debug for SharedContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedContext").finish()
    }
}

impl SharedContext {
    /// Construct from raw wgpu handles (e.g. produced by
    /// `compositor_support_bevy_core_runtime_base::create_wgpu_vulkan_context`).
    pub fn new(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Self {
        Self {
            instance,
            adapter: Arc::new(adapter),
            device: Arc::new(device),
            queue: Arc::new(queue),
        }
    }
}

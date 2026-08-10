//! The worker's fullscreen-pipeline cache.
//!
//! Keyed exactly like the compositor's own shader-pass map, so switching
//! background shaders costs one pipeline build rather than one per frame.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_pipeline_fullscreen_base::fullscreen::FullscreenPass;
use compositor_orchestration_draw_dispatch_frame::ShaderVariant;
use std::collections::HashMap;

#[derive(Default)]
pub struct Passes {
    map: HashMap<(u64, vk::Format), FullscreenPass>,
}

impl Passes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the pass for `(v.id, format)` if absent, then return it.
    pub fn get(
        &mut self,
        dev: &VulkanDevice,
        v: &ShaderVariant,
        format: vk::Format,
    ) -> Result<&FullscreenPass, String> {
        if !self.map.contains_key(&(v.id, format)) {
            let pass = FullscreenPass::create(
                dev,
                format,
                &v.spv,
                v.vert_spv.as_deref(),
                &v.vert_entry,
                &v.frag_entry,
                v.push.len() as u32,
            )
            .map_err(|e| format!("worker fullscreen pass: {e:?}"))?;
            self.map.insert((v.id, format), pass);
        }
        self.map
            .get(&(v.id, format))
            .ok_or_else(|| "worker: pass missing after build".to_string())
    }

    pub fn destroy(&self, dev: &VulkanDevice) {
        for pass in self.map.values() {
            pass.destroy(dev);
        }
    }
}

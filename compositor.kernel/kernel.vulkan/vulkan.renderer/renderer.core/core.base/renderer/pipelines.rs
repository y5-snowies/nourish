//! The per-format pipeline caches (SDR composite + background, HDR composite +
//! background), created on demand and held for the renderer's lifetime.

use ash::vk;
use compositor_kernel_vulkan_pipeline_fullscreen_base::fullscreen::FullscreenPass;

use crate::error::VulkanError;
use crate::frame::ShaderVariant;
use super::VulkanRenderer;

impl VulkanRenderer {
    pub(super) fn ensure_pipelines(&mut self, format: vk::Format) -> Result<(), VulkanError> {
        if !self.pipelines.contains_key(&format) {
            let p = compositor_kernel_vulkan_pipeline_composite_base::composite::create(
                &self.dev,
                self.pipeline_cache,
                format,
            )
            .map_err(|e| VulkanError::Vk(format!("composite pipeline: {e}")))?;
            self.pipelines.insert(format, p);
        }
        Ok(())
    }

    /// Lazily build the HDR composite pipeline for `format`.
    pub(super) fn ensure_hdr_pipeline(&mut self, format: vk::Format) -> Result<(), VulkanError> {
        if self.hdr_pipelines.contains_key(&format) {
            return Ok(());
        }
        let hdr = crate::hdr_composite::HdrComposite::create(
            &self.dev,
            self.phd.handle(),
            self.pipeline_cache,
            format,
        )
        .map_err(|e| VulkanError::Vk(format!("hdr composite pipeline: {e}")))?;
        self.hdr_pipelines.insert(format, hdr);
        info!("vulkan: HDR composite pipeline created for {format:?}");
        Ok(())
    }

    /// Lazily build the generic `FullscreenPass` for a scene shader variant,
    /// keyed by `(variant id, format)`. Built once per (shader, format) and held
    /// for the renderer's lifetime.
    pub(super) fn ensure_shader_pass(
        &mut self,
        v: &ShaderVariant,
        format: vk::Format,
    ) -> Result<(), VulkanError> {
        if self.shader_passes.contains_key(&(v.id, format)) {
            return Ok(());
        }
        let pass = FullscreenPass::create(
            &self.dev,
            format,
            &v.spv,
            v.vert_spv.as_deref(),
            &v.vert_entry,
            &v.frag_entry,
            v.push.len() as u32,
        )?;
        self.shader_passes.insert((v.id, format), pass);
        info!("vulkan: native fullscreen shader pass {} pipeline created for {format:?}", v.id);
        Ok(())
    }
}

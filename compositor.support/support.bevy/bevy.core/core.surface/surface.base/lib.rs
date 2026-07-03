//! `BevySurface`: one DMABUF, imported as both a `wgpu::Texture` and a
//! `GlesTexture`. One per Bevy instance. The engine renders into the wgpu
//! texture; the compositor samples the GLES texture.

use compositor_support_bevy_core_alloc_base::{AllocatedDmabuf, allocate_dmabuf_negotiated};
use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_fault_base::SurfaceError;
use compositor_support_bevy_core_gles_base::import_dmabuf_to_gles;
use compositor_support_bevy_core_import_base::{TEXTURE_FORMAT, import_dmabuf_to_wgpu};
use compositor_developer_debug_instance_record::info;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::{Physical, Size};

/// One render target, addressable from both wgpu and GLES.
///
/// Drop order is load-bearing: `gles_texture` before `wgpu_texture` before
/// `allocated`. Both texture views reference the dmabuf; the allocation owns
/// the GPU memory. Drop fields in declaration order means imports go first.
pub struct BevySurface {
    /// Sampleable view used by the compositor.
    pub gles_texture: GlesTexture,
    /// Render-attachment view used by Bevy.
    pub wgpu_texture: wgpu::Texture,
    /// Underlying allocation.
    pub allocated: AllocatedDmabuf,
    /// Logical size (equals texture extent today).
    pub size: Size<i32, Physical>,
}

impl std::fmt::Debug for BevySurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BevySurface")
            .field("size", &self.size)
            .field("format", &TEXTURE_FORMAT)
            .finish()
    }
}

impl BevySurface {
    /// Allocate a fresh dmabuf at the given size and import it as both views.
    pub fn allocate(
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        info!("BevySurface::allocate {}x{}", size.w, size.h);

        // Negotiate an explicit modifier across gles ∩ wgpu (empty ⇒ implicit path).
        let fourcc = smithay::backend::allocator::Fourcc::Argb8888;
        let mods = compositor_kernel_graphic_bridge_negotiate_base::negotiate::bridge_modifiers(
            smithay::backend::renderer::ImportDma::dmabuf_formats(gles),
            wgpu_ctx.importable.clone(),
            fourcc,
        );
        let allocated =
            allocate_dmabuf_negotiated(render_node, size.w as u32, size.h as u32, fourcc, &mods)?;
        let gles_texture = import_dmabuf_to_gles(gles, &allocated.dmabuf)?;
        let wgpu_texture = import_dmabuf_to_wgpu(wgpu_ctx, &allocated.dmabuf)?;

        Ok(Self {
            gles_texture,
            wgpu_texture,
            allocated,
            size,
        })
    }

    /// Resize: destroy-and-recreate, drop-safe.
    ///
    /// If allocation fails, `*self` is left unchanged so the caller sees a
    /// clean error.
    pub fn resize(
        &mut self,
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        new_size: Size<i32, Physical>,
    ) -> Result<(), SurfaceError> {
        if new_size == self.size {
            return Ok(());
        }
        info!(
            "BevySurface::resize {}x{} -> {}x{}",
            self.size.w, self.size.h, new_size.w, new_size.h
        );

        let replacement = BevySurface::allocate(render_node, wgpu_ctx, gles, new_size)?;
        let _old = std::mem::replace(self, replacement);
        Ok(())
    }

    /// Produce a `wgpu::TextureView` for use as a render attachment.
    pub fn create_render_view(&self) -> wgpu::TextureView {
        self.wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("y5_bevy_dmabuf_render_view"),
            ..Default::default()
        })
    }
}

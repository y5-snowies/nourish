use compositor_support_bevy_core_alloc_base::{AllocatedDmabuf, allocate_dmabuf_negotiated};
use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_fault_base::SurfaceError;
use compositor_support_bevy_core_gles_base::import_dmabuf_to_gles;
use compositor_support_bevy_core_import_base::import_dmabuf_to_wgpu;
use compositor_model_debug_instance_record::trace;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::{Physical, Size};

/// Drop order is load-bearing and Rust drops fields top-down: `gles_texture`
/// holds an EGLImage referencing the dmabuf, `wgpu_texture` holds a Vulkan
/// external-memory binding referencing its fd, and `allocated` owns the memory
/// both point at. Releasing the allocation while either import is alive hits
/// driver assertions. Do not reorder without redoing that analysis.
pub struct Slot {
    /// Sampleable view used by the compositor.
    pub gles_texture: GlesTexture,
    /// Render-attachment view used by Bevy.
    pub wgpu_texture: wgpu::Texture,
    /// Underlying allocation.
    pub allocated: AllocatedDmabuf,
}

impl Slot {
    /// Allocate a fresh dmabuf at `size` and import it as both views.
    pub fn allocate(
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        trace!("bevy slot allocate {}x{}", size.w, size.h);

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

        Ok(Self { gles_texture, wgpu_texture, allocated })
    }

    /// A `wgpu::TextureView` for use as a render attachment.
    pub fn view(&self) -> wgpu::TextureView {
        self.wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("y5_bevy_dmabuf_render_view"),
            ..Default::default()
        })
    }

    pub fn dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf {
        &self.allocated.dmabuf
    }
}

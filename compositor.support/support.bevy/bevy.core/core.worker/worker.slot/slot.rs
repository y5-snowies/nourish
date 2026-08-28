use compositor_support_bevy_core_alloc_base::{AllocatedDmabuf, allocate_dmabuf_negotiated};
use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_fault_base::SurfaceError;
use compositor_support_bevy_core_import_base::import_dmabuf_to_wgpu;
use smithay::utils::{Physical, Size};

/// Drop order is load-bearing: the wgpu texture holds a Vulkan external-memory
/// binding referencing the dmabuf's fd, and `allocated` owns the memory it points
/// at. Fields drop top-down, so the import goes first.
pub struct Slot {
    pub wgpu_texture: wgpu::Texture,
    pub allocated: AllocatedDmabuf,
}

impl Slot {
    pub fn allocate(
        render_node: &str,
        ctx: &WgpuVulkanContext,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        let (fourcc, mods) = compositor_kernel_graphic_format_answer_base::answer::producer_formats(&ctx.formats, compositor_kernel_graphic_format_answer_base::answer::Consumer::BevyWorker);
        let allocated = allocate_dmabuf_negotiated(
            render_node,
            size.w.max(1) as u32,
            size.h.max(1) as u32,
            fourcc,
            &mods,
        )?;
        let wgpu_texture = import_dmabuf_to_wgpu(ctx, &allocated.dmabuf)?;
        Ok(Self { wgpu_texture, allocated })
    }

    pub fn dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf {
        &self.allocated.dmabuf
    }
}

//! A worker-owned ring slot: a dmabuf and its wgpu import, and nothing else.
//!
//! Deliberately NOT [`crate::surface::IcedSurface`]'s backing, which also carries
//! a `GlesTexture`. The off-thread path is Vulkan-only, and there the compositor
//! imports the dmabuf natively rather than sampling a GLES view. Dropping that
//! import is what lets the worker own its buffers outright: the GLES import is
//! the one step that needs `&mut GlesRenderer`, which never leaves the compositor
//! thread.
//!
//! ARGB8888 with an EXPLICIT, negotiated modifier — the same guarantee the
//! inline surfaces get, against a different intersection.
//!
//! Inline slots intersect `gles ∩ wgpu` because they build a `GlesTexture` per
//! slot whatever renderer is compositing. These slots build none: the compositor
//! imports the dmabuf natively, so the constraint is what the COMPOSITING
//! renderer can import (published once at startup by the kernel) intersected
//! with what this wgpu device can. Using the GLES set here would both
//! over-constrain — rejecting modifiers Vulkan takes and GLES does not — and
//! under-constrain, by never checking the renderer that actually samples.
//!
//! An empty intersection falls back to the implicit gbm path, byte-identical to
//! before. That matters: an implicit allocation can report modifier INVALID,
//! which is exactly what the wgpu import refuses on AMD, and it is the case the
//! negotiated path was added for.

use crate::dmabuf_alloc::{AllocatedDmabuf, allocate_dmabuf_negotiated};
use crate::error::SurfaceError;
use crate::wgpu_context::WgpuVulkanContext;
use crate::wgpu_import::import_dmabuf_to_wgpu;
use smithay::utils::{Physical, Size};

/// Drop order is load-bearing: the wgpu texture holds a Vulkan external-memory
/// binding referencing the dmabuf's fd, and `allocated` owns the memory it points
/// at. Fields drop top-down, so the import goes first.
pub struct WorkerSlot {
    pub wgpu_texture: wgpu::Texture,
    pub allocated: AllocatedDmabuf,
}

impl WorkerSlot {
    pub fn allocate(
        render_node: &str,
        ctx: &WgpuVulkanContext,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        let fourcc = smithay::backend::allocator::Fourcc::Argb8888;
        let mods =
            compositor_kernel_graphic_bridge_negotiate_compositor::compositor::worker_modifiers(
                ctx.importable.clone(),
                fourcc,
            );
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

    pub fn view(&self) -> wgpu::TextureView {
        self.wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("y5_iced_worker_render_view"),
            ..Default::default()
        })
    }
}

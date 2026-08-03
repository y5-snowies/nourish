//! `BevySurface`: the ring of dmabufs one Bevy instance renders through.
//!
//! Each entry is a [`Slot`] — one dmabuf imported as both a `wgpu::Texture`
//! (Bevy's render attachment) and a `GlesTexture` (what the compositor samples).
//! The engine draws into [`target`](BevySurface::target); the compositor is only
//! ever shown [`published`](BevySurface::published), a buffer whose write has
//! already completed. Ring depth is the live `Surfaces` setting: one slot is the
//! disabled path and behaves exactly as the single-buffer surface did.

use compositor_kernel_graphic_bridge_publish_ring::ring::Ring;
use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_fault_base::SurfaceError;
use compositor_support_bevy_core_slot_base::Slot;
use compositor_model_debug_instance_record::info;
use compositor_model_environment_interface_base::base as interface;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::{Physical, Size};

pub struct BevySurface {
    ring: Ring<Slot>,
    /// Logical size (equals every slot's texture extent).
    pub size: Size<i32, Physical>,
}

impl std::fmt::Debug for BevySurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BevySurface")
            .field("size", &self.size)
            .field("slots", &self.ring.len())
            .finish()
    }
}

impl BevySurface {
    /// Allocate the ring at the depth the live setting asks for.
    pub fn allocate(
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        size: Size<i32, Physical>,
    ) -> Result<Self, SurfaceError> {
        let depth = interface::get().depth();
        info!("BevySurface::allocate {}x{} slots={depth}", size.w, size.h);
        let mut slots = Vec::with_capacity(depth);
        for _ in 0..depth {
            slots.push(Slot::allocate(render_node, wgpu_ctx, gles, size)?);
        }
        let ring = Ring::new(slots, wgpu_ctx.device.clone(), wgpu_ctx.queue.clone());
        Ok(Self { ring, size })
    }

    /// Match the ring to the live setting, allocating or dropping slots. Called
    /// per frame, so toggling the setting applies without a restart.
    pub fn sync_depth(
        &mut self,
        render_node: &str,
        wgpu_ctx: &WgpuVulkanContext,
        gles: &mut GlesRenderer,
        want: usize,
    ) -> Result<(), SurfaceError> {
        if want == self.ring.len() {
            return Ok(());
        }
        info!("BevySurface ring {} -> {want} slots", self.ring.len());
        let size = self.size;
        self.ring.set_depth(want, || Slot::allocate(render_node, wgpu_ctx, gles, size))
    }

    /// Resize: destroy-and-recreate every slot. Allocated up front so a failure
    /// leaves `*self` untouched and the caller sees a clean error.
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
        let mut slots = Vec::with_capacity(self.ring.len());
        for _ in 0..self.ring.len() {
            slots.push(Slot::allocate(render_node, wgpu_ctx, gles, new_size)?);
        }
        self.ring.replace(slots);
        self.size = new_size;
        Ok(())
    }

    /// Retire finished frames. Returns whether a new buffer became visible, which
    /// is what the instance's commit counter — and so its damage — follows.
    pub fn poll(&mut self) -> bool {
        self.ring.poll()
    }

    /// Claim the slot to render into. May block once the GPU is a whole ring behind.
    pub fn begin(&mut self) -> &Slot {
        self.ring.begin()
    }

    /// Whether Bevy must be re-pointed at [`target`](Self::target) before it
    /// draws. False on a single-slot ring, which never rotates.
    pub fn take_retarget(&mut self) -> bool {
        self.ring.take_retarget()
    }

    /// Record that Bevy submitted its frame for the claimed slot.
    ///
    /// `pipeline` is passed in rather than re-read: the caller already holds the
    /// settings for this frame, and re-reading the global here made it one
    /// `RwLock` acquisition per surface per frame to learn a value that had not
    /// changed since the frame began.
    pub fn submitted(&mut self, pipeline: bool) {
        self.ring.submitted(pipeline);
    }

    /// The slot Bevy is currently drawing into.
    pub fn target(&self) -> &Slot {
        self.ring.target()
    }

    /// The slot the compositor samples — never one still being written.
    pub fn published(&self) -> &Slot {
        self.ring.published()
    }

    pub fn gles_texture(&self) -> &GlesTexture {
        &self.published().gles_texture
    }

    pub fn dmabuf(&self) -> &smithay::backend::allocator::dmabuf::Dmabuf {
        self.published().dmabuf()
    }

    pub fn generation(&self) -> u64 {
        self.ring.generation()
    }
}

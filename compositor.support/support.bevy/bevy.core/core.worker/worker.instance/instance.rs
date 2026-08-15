use compositor_support_bevy_core_context_base::WgpuVulkanContext;
use compositor_support_bevy_core_fault_base::SurfaceError;
use compositor_support_bevy_core_handle_base::HandleId;
use compositor_support_bevy_core_instance_base::BevyInstanceAny;
use compositor_support_bevy_core_publish_base::Published;
use compositor_support_bevy_core_worker_base::{Job, Worker};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Physical, Point, Size};
use std::any::Any;

pub struct WorkerInstance {
    pub id: HandleId,
    pub smithay_id: Id,
    pub commit: CommitCounter,
    pub location: Point<i32, Physical>,
    /// The size the compositor has ASKED for. The published frame may still be at
    /// the previous one; the element follows the published size, not this.
    pub size: Size<i32, Physical>,
    pub scale_factor: f32,
    pub pending_resize: Option<(Size<i32, Physical>, f32)>,
    pub generation: u64,
    pub worker: Worker,
    /// Newest finished frame, refreshed each tick. `None` until the worker has
    /// published once, and the instance simply draws nothing until then.
    pub published: Option<Published>,
}

impl BevyInstanceAny for WorkerInstance {
    fn handle_id(&self) -> HandleId { self.id }
    fn smithay_id(&self) -> &Id { &self.smithay_id }
    fn commit(&self) -> CommitCounter { self.commit }
    fn bump_commit(&mut self) { self.commit.increment(); }
    fn location(&self) -> Point<i32, Physical> { self.location }
    fn size(&self) -> Size<i32, Physical> { self.size }
    fn set_location(&mut self, p: Point<i32, Physical>) { self.location = p; }

    /// No rendering here — the worker is already advancing on its own tick. This
    /// only picks up whatever it has finished, and damage follows that.
    fn tick(&mut self) {
        let latest = self.worker.board().get(self.id);
        let generation = latest.as_ref().map_or(0, |p| p.generation);
        self.published = latest;
        if generation != self.generation {
            self.generation = generation;
            self.commit.increment();
        }
    }

    /// Vulkan-only path: the compositor imports `dmabuf` natively and never reads
    /// a GLES view, so the worker never builds one.
    fn texture_handle(&self) -> Option<&GlesTexture> { None }

    fn dmabuf(&self) -> Option<Dmabuf> {
        self.published.as_ref().map(|p| p.dmabuf.clone())
    }

    /// The size actually rendered, which is what the buffer holds. Using the
    /// requested size while a resize is still in flight would sample past its end.
    fn rendered_size(&self) -> Option<Size<i32, Physical>> {
        self.published.as_ref().map(|p| p.size)
    }

    fn apply_pending_resize(&mut self, _node: &str, _ctx: &WgpuVulkanContext, _gles: &mut GlesRenderer) -> Result<bool, SurfaceError> {
        let Some((size, scale)) = self.pending_resize.take() else { return Ok(false) };
        if size == self.size && scale == self.scale_factor {
            return Ok(false);
        }
        self.size = size;
        self.scale_factor = scale;
        self.worker.send(Job::Resize { id: self.id, size, scale });
        self.commit.increment();
        Ok(true)
    }

    /// Ring depth is read by the worker itself, which owns the buffers.
    fn sync_depth(&mut self, _node: &str, _ctx: &WgpuVulkanContext, _gles: &mut GlesRenderer, _want: usize) -> Result<(), SurfaceError> { Ok(()) }

    fn request_resize(&mut self, new_size: Size<i32, Physical>, scale_factor: f32) {
        self.pending_resize = Some((new_size, scale_factor));
    }
    fn pending_resize(&self) -> Option<(Size<i32, Physical>, f32)> { self.pending_resize }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

impl Drop for WorkerInstance {
    fn drop(&mut self) {
        self.worker.send(Job::Destroy(self.id));
    }
}

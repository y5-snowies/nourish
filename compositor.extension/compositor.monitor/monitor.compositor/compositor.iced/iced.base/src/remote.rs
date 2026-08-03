//! The compositor-side stand-in for an iced instance living on the worker.
//!
//! Implements the same `IcedInstanceAny` vtable as the inline `IcedInstance`, so
//! the registry, item wrapper and element builder do not care which they hold.
//! Owns only bookkeeping — id, placement, size, commit — and reads finished
//! frames off the `Board`. No `IcedRuntime`, no GPU resource: the worker has both.

use crate::handle::HandleId;
use crate::instance::IcedInstanceAny;
use crate::publish::Published;
use crate::worker::{Job, Worker};
use compositor_monitor_runtime_surface_base::{SurfaceError, WgpuVulkanContext};
use iced_core::{Event as IcedEvent, mouse};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Physical, Point, Size};
use std::any::Any;

pub struct RemoteInstance {
    pub id: HandleId,
    pub smithay_id: Id,
    pub commit: CommitCounter,
    pub location: Point<i32, Physical>,
    /// What the compositor has ASKED for; the published frame may still be at the
    /// previous size, and the element follows that instead.
    pub size: Size<i32, Physical>,
    pub scale_factor: f32,
    pub pending_resize: Option<(Size<i32, Physical>, f32)>,
    /// The world location sent with the most recent `Job::Resize`.
    ///
    /// The difference between this and `location` is movement that owes nothing
    /// to a resize — a plain drag of the surface — and must apply NOW rather
    /// than wait for a frame. See `rendered_location`.
    pub resize_location: Point<i32, Physical>,
    pub generation: u64,
    pub worker: Worker,
    pub published: Option<Published>,
    /// Mirrors what we last told the worker, so visibility is only re-sent when
    /// it actually flips rather than every frame.
    pub resident: bool,
}

impl IcedInstanceAny for RemoteInstance {
    fn handle_id(&self) -> HandleId { self.id }
    fn smithay_id(&self) -> &Id { &self.smithay_id }
    fn commit(&self) -> CommitCounter { self.commit }
    fn bump_commit(&mut self) { self.commit.increment(); }
    fn location(&self) -> Point<i32, Physical> { self.location }
    fn size(&self) -> Size<i32, Physical> { self.size }
    fn scale_factor(&self) -> f32 { self.scale_factor }
    fn set_location(&mut self, p: Point<i32, Physical>) {
        if self.location != p {
            self.location = p;
            self.commit.increment();
        }
    }

    fn queue_event(&mut self, event: IcedEvent) {
        self.worker.send(Job::Event { id: self.id, event });
    }

    /// Nothing to advance here — the worker ticks on its own. Returning false
    /// keeps the registry from calling `render()`, which is also a no-op.
    fn tick(&mut self) -> bool { false }
    fn acknowledge_frame(&mut self) {}

    /// The worker reports this for the whole registry; per-instance it would mean
    /// round-tripping, which is exactly what this path exists to avoid.
    fn wants_frame(&self) -> bool { self.worker.board().wants() }

    fn render(&mut self) {}

    /// Pick up whatever the worker finished. Damage follows the ring generation,
    /// so a frame that published nothing reports none.
    fn poll_publish(&mut self) {
        let latest = self.worker.board().get(self.id);
        let generation = latest.as_ref().map_or(0, |p| p.generation);
        self.published = latest;
        if generation != self.generation {
            self.generation = generation;
            self.commit.increment();
        }
    }

    fn is_resident(&self) -> bool { self.resident }

    fn release_backing(&mut self) {
        if self.resident {
            self.resident = false;
            self.worker.send(Job::Visible { id: self.id, visible: false });
        }
    }

    fn ensure_backing(&mut self, _node: &str, _ctx: &WgpuVulkanContext, _gles: &mut GlesRenderer) -> Result<(), SurfaceError> {
        if !self.resident {
            self.resident = true;
            self.worker.send(Job::Visible { id: self.id, visible: true });
        }
        Ok(())
    }

    /// Ring depth is read by the worker, which owns the buffers.
    fn sync_depth(&mut self, _node: &str, _ctx: &WgpuVulkanContext, _gles: &mut GlesRenderer, _want: usize) -> Result<(), SurfaceError> { Ok(()) }

    /// Vulkan-only path: the compositor imports `dmabuf` natively and never reads
    /// a GLES view, so the worker never builds one.
    fn texture_handle(&self) -> Option<&GlesTexture> { None }

    fn dmabuf(&self) -> Option<Dmabuf> {
        self.published.as_ref().map(|p| p.dmabuf.clone())
    }

    fn rendered_size(&self) -> Option<Size<i32, Physical>> {
        self.published.as_ref().map(|p| p.size)
    }

    /// The world location to draw the published frame at.
    ///
    /// `published.location` is the origin that frame was rendered for, so
    /// pairing it with `published.size` reproduces exactly the rect iced laid
    /// out — the anchored edge cannot move, and only the dragged edge trails.
    ///
    /// The correction term is what keeps a plain MOVE immediate: a move sends no
    /// resize, so `published.location` would sit at the last resize's origin and
    /// the surface would refuse to follow the cursor. `location - resize_location`
    /// is exactly the movement since that resize which the resize did not cause,
    /// and it is zero during a drag (every step sends one).
    fn rendered_location(&self) -> Option<Point<i32, Physical>> {
        let p = self.published.as_ref()?;
        Some(p.location + (self.location - self.resize_location))
    }

    fn apply_pending_resize(&mut self, _node: &str, _ctx: &WgpuVulkanContext, _gles: &mut GlesRenderer) -> Result<bool, SurfaceError> {
        let Some((size, scale)) = self.pending_resize.take() else { return Ok(false) };
        if size == self.size && scale == self.scale_factor {
            return Ok(false);
        }
        self.size = size;
        self.scale_factor = scale;
        // Send the origin WITH the size: they are one geometry, and the frame
        // must come back knowing which origin it was laid out for.
        //
        // ORDER IS LOAD-BEARING and currently correct by accident of the caller:
        // `request_resize` only QUEUES, while `set_location` applies at once, and
        // the queue is flushed here on the next frame — so `self.location` is
        // already the origin that belongs with `size`. Flush a resize before its
        // matching `set_location` and the two would pair wrongly: the origin
        // move would be charged to the correction term in `rendered_location`,
        // applied immediately, and the jitter would come back silently.
        self.resize_location = self.location;
        self.worker.send(Job::Resize { id: self.id, size, scale, location: self.location });
        self.commit.increment();
        Ok(true)
    }

    fn request_resize(&mut self, new_size: Size<i32, Physical>, scale_factor: f32) {
        self.pending_resize = Some((new_size, scale_factor));
    }
    fn pending_resize(&self) -> Option<(Size<i32, Physical>, f32)> { self.pending_resize }

    fn pointer_leave(&mut self) {
        self.queue_event(IcedEvent::Mouse(mouse::Event::CursorLeft));
    }

    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

impl Drop for RemoteInstance {
    fn drop(&mut self) {
        self.worker.send(Job::Destroy(self.id));
    }
}

//! `VulkanTexture` — a reference-counted sampled image. The GPU resources live
//! behind an `Arc<TextureInner>` (in the `image.inner` sibling) so smithay can
//! clone `TextureId` freely. Fields/accessors are `pub` so the renderer, the
//! SHM upload path, and the per-surface cache can all share the type.

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::{Dmabuf, WeakDmabuf};
use smithay::backend::renderer::Texture;
use smithay::utils::{Buffer as BufferCoord, Size};
use std::sync::Arc;

pub use compositor_kernel_vulkan_texture_image_inner::inner::TextureInner;

#[derive(Debug, Clone)]
pub struct VulkanTexture {
    pub inner: Arc<TextureInner>,
    /// Per-surface HDR composite flag `[transfer, is_hdr, 0, 0]`; default SDR.
    pub surf: [f32; 4],
    /// The client dmabuf this was imported from, kept so a SECOND device can
    /// import the same buffer. `None` for a SHM surface, which has no fd at all —
    /// the compositor uploaded it into a device-local image instead.
    ///
    /// Retaining the source is what lets the background worker sample window
    /// content without the compositor re-exporting anything: its memory was not
    /// allocated exportable, so `vkGetMemoryFdKHR` is not available on it, but the
    /// ORIGINAL fd still is. The worker dups it on import, so its copy stays valid
    /// even after the client releases the buffer.
    ///
    /// WEAK, and that is load-bearing. `import_cache` is keyed by `WeakDmabuf` and
    /// evicted with `retain(|w, _| w.upgrade().is_some())`, so a cached texture
    /// holding a STRONG clone of its own key keeps that key alive: the entry can
    /// never be reaped, the cache grows without bound, and every client buffer
    /// ever imported is pinned for the life of the process. A weak reference
    /// answers the same question — "can a second device still reach these pixels?"
    /// — and answers it correctly when the client has released them.
    pub source: Option<WeakDmabuf>,
    /// For a SHM surface: this image shared as an `OPAQUE_FD`, so the worker can
    /// reconstruct it on its own logical device. `Arc` because the set is cloned
    /// on every read and the fd must be owned exactly once.
    ///
    /// Exactly one of `source` / `shared` is set. dmabuf clients hand on the
    /// client's own fd; SHM clients have none, so the compositor shares the image
    /// it uploaded into instead.
    pub shared: Option<std::sync::Arc<compositor_kernel_vulkan_memory_external_base::external::Shared>>,
}

impl VulkanTexture {
    pub fn new(inner: Arc<TextureInner>) -> Self {
        Self { inner, surf: [0.0; 4], source: None, shared: None }
    }
    pub fn view(&self) -> ash::vk::ImageView {
        self.inner.view
    }
    pub fn surf(&self) -> [f32; 4] {
        self.surf
    }
    pub fn set_surf(&mut self, surf: [f32; 4]) {
        self.surf = surf;
    }
}

impl Texture for VulkanTexture {
    fn width(&self) -> u32 {
        self.inner.width
    }
    fn height(&self) -> u32 {
        self.inner.height
    }
    fn size(&self) -> Size<i32, BufferCoord> {
        Size::from((self.inner.width as i32, self.inner.height as i32))
    }
    fn format(&self) -> Option<Fourcc> {
        self.inner.fourcc
    }
}

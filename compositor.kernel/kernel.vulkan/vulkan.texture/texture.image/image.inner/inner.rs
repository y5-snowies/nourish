use ash::vk;
use smithay::backend::allocator::Fourcc;
use std::fmt;
use std::sync::{Arc, Mutex};

/// GPU handles whose owner has dropped but whose last-using frame may still be in
/// flight. Destroyed at the renderer's drain point, where the previous frame is
/// provably complete — the same rule `RetiredTarget` already follows for render
/// targets, applied to textures.
pub struct RetiredTexture {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub extra_memory: Vec<vk::DeviceMemory>,
    pub view: vk::ImageView,
    pub owns_memory: bool,
}

/// GPU resources behind `VulkanTexture`'s `Arc`; destroyed when the last
/// handle drops. dmabuf-imported memory is owned by us (fd dup'd at import).
pub struct TextureInner {
    pub device: ash::Device,
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    /// Extra per-plane allocations for a DISJOINT multi-plane (Intel CCS)
    /// import; empty for the common single-memory case. Freed with `memory`.
    pub extra_memory: Vec<vk::DeviceMemory>,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub fourcc: Option<Fourcc>,
    pub width: u32,
    pub height: u32,
    pub owns_memory: bool,
    /// Where to defer destruction to. `None` destroys inline, which is only correct
    /// when no frame can still reference the image.
    ///
    /// Destroying inline is what made a dropped-but-in-flight texture a
    /// use-after-free: an imported surface that a frame acquired and did not draw is
    /// owned only by `import_cache`, and `reap_targets()` drops that entry as soon as
    /// the client's dmabuf dies — freeing the image while the just-submitted command
    /// buffer still referenced it (NVIDIA `Xid 31`, MMU FAULT_PTE).
    pub retire: Option<Arc<Mutex<Vec<RetiredTexture>>>>,
}

impl fmt::Debug for TextureInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextureInner")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("fourcc", &self.fourcc)
            .finish()
    }
}

impl Drop for TextureInner {
    fn drop(&mut self) {
        // Hand the handles to the renderer rather than destroying them here: this can
        // run at any point in a frame, including while the GPU is still reading the
        // image.
        if let Some(q) = self.retire.take() {
            if let Ok(mut q) = q.lock() {
                q.push(RetiredTexture {
                    image: self.image,
                    memory: self.memory,
                    extra_memory: std::mem::take(&mut self.extra_memory),
                    view: self.view,
                    owns_memory: self.owns_memory,
                });
                return;
            }
        }
        unsafe {
            if self.view != vk::ImageView::null() {
                self.device.destroy_image_view(self.view, None);
            }
            if self.image != vk::Image::null() {
                self.device.destroy_image(self.image, None);
            }
            if self.owns_memory && self.memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.memory, None);
                for m in &self.extra_memory {
                    self.device.free_memory(*m, None);
                }
            }
        }
    }
}

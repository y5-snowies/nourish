//! One render slot: the image the worker draws into, plus the dmabuf the
//! compositor samples. Both name the SAME underlying memory.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::Dmabuf;

pub struct Target {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
    pub format: vk::Format,
    pub dmabuf: Dmabuf,
}

impl Target {
    /// Free the worker-side Vulkan objects. The [`Dmabuf`] is a standalone kernel
    /// object and stays valid afterwards, so it is dropped separately.
    pub fn destroy(self, dev: &VulkanDevice) {
        unsafe {
            dev.device.destroy_image_view(self.view, None);
            dev.device.destroy_image(self.image, None);
            dev.device.free_memory(self.memory, None);
        }
    }
}

/// Fixed for the worker's lifetime. Everything tunable lives in the
/// process-global `TripleBuffer`, which the worker re-reads each pass — that is
/// what lets the knobs apply live without a respawn.
pub struct Config {
    pub fourcc: Fourcc,
}

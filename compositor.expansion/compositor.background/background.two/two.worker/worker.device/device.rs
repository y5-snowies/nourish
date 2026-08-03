//! The worker's own Vulkan device.
//!
//! Deliberately a SECOND logical device on its own instance: the compositor
//! blocks on its single command buffer's fence every frame and renders inline on
//! the calloop thread that also dispatches input, so a heavy shader in that
//! submission surfaces as input latency. Nothing here is shared with it.
//!
//! The device owns the POOL; the command buffers and fences belong to PANES.
//! They used to be a fixed ring of three here, handed out round-robin across
//! every pane, which coupled panes that share nothing else: with four panes
//! pipelining, a pane's `acquire` waited on a fence belonging to another
//! monitor's frame, and the depth a pane actually got depended on how many other
//! panes happened to be live. A pane's command slot is now its buffer slot, so
//! each pane gets the depth it was configured with whatever else is on screen.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;
use compositor_kernel_vulkan_device_queue_base::queue::{self, RenderQueue};
use smithay::backend::vulkan::PhysicalDevice;

pub struct Device {
    pub dev: VulkanDevice,
    pub phd: PhysicalDevice,
    pub queue: RenderQueue,
    pool: vk::CommandPool,
}

impl Device {
    pub fn new() -> Result<Self, String> {
        let instance = compositor_kernel_vulkan_instance_factory_base::factory::create()
            .map_err(|e| format!("worker instance: {e:?}"))?;
        // The compositor's OWN node, never just the first device — see the crate.
        let phd = compositor_background_two_worker_physical::physical::select(&instance)?;
        info!("background worker: physical device '{}'", phd.name());
        let dev = compositor_kernel_vulkan_device_factory_base::factory::create(&phd)
            .map_err(|e| format!("worker device: {e}"))?;
        let queue = queue::graphics_queue(&dev);
        let pool = compositor_kernel_vulkan_command_pool_base::pool::create(&dev)
            .map_err(|e| format!("worker command pool: {e}"))?;
        Ok(Self { dev, phd, queue, pool })
    }

    /// A pane's own command ring — see `worker.ring` for why it is per-pane.
    pub fn alloc_ring(
        &self,
        slots: usize,
    ) -> Result<(Vec<vk::CommandBuffer>, Vec<vk::Fence>), String> {
        compositor_background_two_worker_ring::ring::alloc(&self.dev, self.pool, slots)
    }

    pub fn free_ring(&self, cmds: &[vk::CommandBuffer], fences: &[vk::Fence]) {
        compositor_background_two_worker_ring::ring::free(&self.dev, self.pool, cmds, fences);
    }

    /// Block until `fence`'s previous submission retired, then re-arm it.
    pub fn acquire(&self, fence: vk::Fence) -> Result<(), String> {
        unsafe {
            self.dev.device.wait_for_fences(&[fence], true, u64::MAX)
                .map_err(|e| format!("worker fence wait: {e}"))?;
            self.dev.device.reset_fences(&[fence])
                .map_err(|e| format!("worker fence reset: {e}"))
        }
    }

    /// Has `fence` retired? Non-blocking — the pipelined path polls this, since
    /// waiting one pass later only relocates the stall.
    pub fn signalled(&self, fence: vk::Fence) -> bool {
        unsafe { self.dev.device.get_fence_status(fence) }.unwrap_or(true)
    }

    /// Block until `fence` retired, leaving it signalled for `acquire`.
    pub fn complete(&self, fence: vk::Fence) -> Result<(), String> {
        unsafe {
            self.dev.device.wait_for_fences(&[fence], true, u64::MAX)
                .map_err(|e| format!("worker fence wait: {e}"))
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = self.dev.device.device_wait_idle();
            self.dev.device.destroy_command_pool(self.pool, None);
        }
    }
}

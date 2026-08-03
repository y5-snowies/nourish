//! A pane's command buffers and fences, one pair per ring slot.
//!
//! PER PANE, not shared. These used to be a fixed ring of three on the device,
//! handed out round-robin across every pane, which coupled panes that share
//! nothing else: a pane's `acquire` could wait on a fence belonging to another
//! monitor's frame, and the depth a pane actually got depended on how many other
//! panes happened to be live. A pane's command slot is now its buffer slot, so
//! each pane gets the depth it was configured with whatever else is on screen.

use ash::vk;
use compositor_kernel_vulkan_device_factory_base::factory::VulkanDevice;

/// One command buffer and one fence per slot. Fences start SIGNALED so the first
/// `acquire` on each passes.
pub fn alloc(
    dev: &VulkanDevice,
    pool: vk::CommandPool,
    slots: usize,
) -> Result<(Vec<vk::CommandBuffer>, Vec<vk::Fence>), String> {
    let info = vk::CommandBufferAllocateInfo::default()
        .command_pool(pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(slots as u32);
    let cmds = unsafe { dev.device.allocate_command_buffers(&info) }
        .map_err(|e| format!("worker command buffers: {e}"))?;
    let fi = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
    let mut fences = Vec::with_capacity(slots);
    for _ in 0..slots {
        match unsafe { dev.device.create_fence(&fi, None) } {
            Ok(f) => fences.push(f),
            Err(e) => {
                free(dev, pool, &cmds, &fences);
                return Err(format!("worker fence: {e}"));
            }
        }
    }
    Ok((cmds, fences))
}

/// Release a pane's command buffers and fences. Waits first: a fence may still
/// be attached to work in flight, and freeing under the GPU is undefined
/// behaviour rather than merely late.
pub fn free(
    dev: &VulkanDevice,
    pool: vk::CommandPool,
    cmds: &[vk::CommandBuffer],
    fences: &[vk::Fence],
) {
    unsafe {
        if !fences.is_empty() {
            let _ = dev.device.wait_for_fences(fences, true, u64::MAX);
        }
        if !cmds.is_empty() {
            dev.device.free_command_buffers(pool, cmds);
        }
        for f in fences {
            dev.device.destroy_fence(*f, None);
        }
    }
}

//! Record and submit one fullscreen shader pass into a slot.
//!
//! The queue-family ownership rules around it live in `worker.barrier`.
//!
//! Does NOT wait for completion — the caller owns that, because whether it waits
//! now or one frame later is the pipelining choice. What the caller must not do
//! is publish before the fence signals: the compositor could then sample a buffer
//! still being written, and a write fence on its `dma_resv` would drag the
//! compositor's own submission into waiting on us, re-coupling the two through
//! implicit sync.

use ash::vk;
use compositor_background_two_worker_device::device::Device;
use compositor_background_two_worker_pipeline::pipeline::Passes;
use compositor_background_two_worker_target::target::Target;
use compositor_orchestration_draw_dispatch_frame::ShaderVariant;

pub fn render(
    d: &Device,
    passes: &mut Passes,
    target: &Target,
    v: &ShaderVariant,
    extent: (u32, u32),
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
) -> Result<(), String> {
    let pass = passes.get(&d.dev, v, target.format)?;
    d.acquire(fence)?;
    let dev = &d.dev.device;
    unsafe {
        dev.begin_command_buffer(
            cmd,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )
        .map_err(|e| format!("worker begin: {e}"))?;
    }

    compositor_background_two_worker_barrier::barrier::enter(&d.dev, cmd, target.image);

    // Clear to transparent black: the fullscreen pipeline blends premultiplied-
    // over, and over a zero destination that is an identity, so the slot ends up
    // holding exactly what the shader emitted.
    compositor_kernel_vulkan_pipeline_composite_base::composite::begin(
        &d.dev, cmd, target.view, extent, [0.0; 4], vk::AttachmentLoadOp::CLEAR,
    );
    pass.draw(&d.dev, cmd, &v.push);
    compositor_kernel_vulkan_pipeline_composite_base::composite::end(&d.dev, cmd);

    compositor_background_two_worker_barrier::barrier::leave(&d.dev, cmd, target.image);

    unsafe {
        dev.end_command_buffer(cmd).map_err(|e| format!("worker end: {e}"))?;
        let ci = vk::CommandBufferSubmitInfo::default().command_buffer(cmd);
        let submit = vk::SubmitInfo2::default().command_buffer_infos(std::slice::from_ref(&ci));
        dev.queue_submit2(d.queue.queue, &[submit], fence)
            .map_err(|e| format!("worker submit: {e}"))?;
    }
    Ok(())
}

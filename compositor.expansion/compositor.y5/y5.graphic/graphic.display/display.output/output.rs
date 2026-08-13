use std::os::unix::raw::dev_t;

use compositor_y5_graphic_display_backend::backend::Backend;
use smithay::{output::Output, wayland::dmabuf::DmabufFeedbackBuilder};
use compositor_orchestration_core_state_base::Loop;

pub fn register(_loop: &mut Loop, output: &Output) {
    let _global = output.create_global::<compositor_support_smithay_dispatch_state_base::state::Dispatch>(&_loop.state.output.display_handle);

    // Every world's Space, not just the hosted one — a world parked when a monitor
    // appears would otherwise never learn about it (`map_output_everywhere`).
    _loop.inner.map_output_everywhere(output, smithay::utils::Point::from((0, 0)));
}

pub fn register_dmabuf(_loop: &mut Loop, backend_loader: &mut dyn Backend) {
    let dma_formats = backend_loader.bind_display(&_loop.state.output.display_handle);
    // WHAT CLIENTS ARE ACTUALLY OFFERED. This is the driver's EGL import list, not
    // a list of ours, so the only honest way to know whether 10-bit or planar
    // formats reach clients on a given machine is to print what was advertised.
    {
        let mut codes: Vec<String> =
            dma_formats.iter().map(|f| format!("{:?}", f.code)).collect();
        codes.sort();
        codes.dedup();
        info!(
            "dmabuf feedback: advertising {} format/modifier pair(s) over {} fourcc(s): {}",
            dma_formats.iter().count(),
            codes.len(),
            codes.join(", ")
        );
    }

    if _loop.inner.kernel.get(&compositor_orchestration_core_state_base::state::GPU_BINDING).is_some() {
        info!("Creating DMABuf global v5");
        // The device that will actually SAMPLE these buffers — the composite node,
        // which is `render_node` when it names something usable. It used to be the
        // scanout device unconditionally, which was right only while the two were
        // the same: with the composite moved to `render_node`, advertising the
        // scanout device told clients to allocate on a GPU the sampler is not on.
        let scanout = _loop.inner.kernel.get(&compositor_orchestration_core_state_base::state::GPU_BINDING)
            .as_ref()
            .unwrap()
            .borrow()
            .primary;
        let main_device: dev_t = compositor_kernel_graphic_bridge_negotiate_compositor::compositor::composite_node(scanout).dev_id();

        //
        // The FORMATS are the GLES EGL import set, DELIBERATELY, even when the
        // compositor goes on to composite with Vulkan. It is tempting to
        // republish the Vulkan renderer's set once that is known — it is the
        // renderer that samples, after all — but its `dmabuf_formats()` covers
        // only ARGB/XRGB/ABGR/XBGR 8888, because `vk_format` maps nothing else
        // and there is no YUV sampling anywhere in that renderer. Advertising it
        // would drop NV12 and every planar format from the feedback, which is
        // what a hardware-decoded video client asks for.
        //
        // So this set is WIDER than the Vulkan draw path can import, and that is
        // a real inconsistency: such a buffer is accepted at validation (which
        // goes through GLES) and then fails at draw. Narrowing the advertisement
        // is the wrong half to fix — the right one is either YUV support in the
        // Vulkan renderer or a GLES route for formats it cannot take.
        let default_feedback = DmabufFeedbackBuilder::new(main_device, dma_formats)
            .build()
            .unwrap_or_else(|e| abort!("Failed to build dmabuf feedback: {e:?}"));

        _loop.state.dmabuf.global = Some(
            _loop
                .state
                .dmabuf
                .state
                .create_global_with_default_feedback::<compositor_support_smithay_dispatch_state_base::state::Dispatch>(
                    &_loop.state.output.display_handle,
                    &default_feedback,
                ),
        );
    } else {
        warn!("Creating DMABuf global V3 instead of V5 because primary gpu is not set");
        _loop.state.dmabuf.global = Some(
            _loop
                .state
                .dmabuf
                .state
                .create_global::<compositor_support_smithay_dispatch_state_base::state::Dispatch>(&_loop.state.output.display_handle, dma_formats),
        );
    }
}

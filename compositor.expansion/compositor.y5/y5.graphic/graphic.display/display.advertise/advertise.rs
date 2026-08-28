//! The `zwp_linux_dmabuf_v1` advertisement — what this compositor tells clients
//! it can import.
//!
//! Its own crate because it is a DIFFERENT PHASE from `display.output`'s
//! registration, not a different function in the same one. `bind_display` there
//! publishes a capability; this consumes every capability the machine published
//! and commits to an answer. They ran back to back in one function, and that is
//! exactly what put an answer in the middle of the registration phase — the
//! feedback was built while the wgpu adapters were still probing, from a
//! registrar that was not finished. Keeping the two in separate crates makes the
//! phase boundary something a reader trips over rather than something they have
//! to already know.

use std::os::unix::raw::dev_t;

use smithay::wayland::dmabuf::DmabufFeedbackBuilder;
use compositor_kernel_graphic_format_answer_base::answer;
use compositor_kernel_graphic_format_registrar_base::registrar;
use compositor_kernel_graphic_format_role_base::role::Role;
use compositor_orchestration_core_state_base::Loop;

/// Build the `zwp_linux_dmabuf_v1` global from the format layer's answer.
///
/// Called from the LOADER, after every backend registration and after the wgpu
/// contexts have arrived — i.e. once the registrar is complete. Nothing is lost
/// by the delay: the event loop has not started dispatching, so no client can
/// have seen the registry yet.
pub fn advertise_dmabuf(_loop: &mut Loop) {
    let formats = _loop.inner.kernel.get(&compositor_kernel_graphic_format_registrar_base::registrar::FORMATS).clone();
    // The baseline for the log below, read back from the registrar rather than
    // threaded through from `bind_display` — it is the same set, and the layer is
    // where a capability set is supposed to be looked up.
    let egl_formats = formats.view().set_or_empty(Role::GlesSample);
    // NARROWED TO WHAT WILL ACTUALLY BE IMPORTED. `bind_display` returns mesa's
    // EGL import list, not a list of ours, and the renderer that DRAWS client
    // buffers may be Vulkan, which takes a subset: advertising the EGL list
    // wholesale is how an fp16 (XB4H) client got told yes and then had every
    // frame dropped at draw. The format layer answers with what the ACTIVE renderer
    // registered, minus what the composite cannot EXPRESS (fp16 outside colour
    // management) — policy, so it applies on the gles path too.
    let dma_formats = match answer::available(&formats, answer::Consumer::ClientFeedback) {
        answer::Answer::Set { set, .. } => set,
        _ => smithay::backend::allocator::format::FormatSet::default(),
    };
    // Both halves: what reaches clients on a machine is only knowable from this.
    {
        let codes = |set: &smithay::backend::allocator::format::FormatSet| {
            let mut v: Vec<String> = set.iter().map(|f| format!("{:?}", f.code)).collect();
            v.sort();
            v.dedup();
            v
        };
        let offered = codes(&dma_formats);
        let held: Vec<String> = codes(&egl_formats).into_iter().filter(|c| !offered.contains(c)).collect();
        info!(
            "dmabuf feedback: advertising {} pair(s) over {} fourcc(s): {} || withheld (EGL takes \
             them; the composite cannot import or cannot express them): {}",
            dma_formats.iter().count(), offered.len(), offered.join(", "),
            if held.is_empty() { "none".to_string() } else { held.join(", ") },
        );
    }

    // v5 (per-client feedback naming a main_device) when a GPU binding exists,
    // v3 (a flat format list) when it does not.
    //
    // `main_device` is the device that will actually SAMPLE these buffers — the
    // composite node, `render_node` when usable. Advertising the scanout device
    // instead told clients to allocate on a GPU the sampler is not on.
    //
    // What the narrowing costs on the Vulkan path is YUV: `format.table`
    // classifies every planar code as `Chroma` — importable only through a
    // ycbcr-conversion sampler the renderer does not build — so NV12 and friends
    // are not offered while Vulkan composites, and a hardware-decoded video client
    // negotiates RGB (or converts) instead of handing over a buffer that would
    // never be drawn. Getting them back is ycbcr support in the Vulkan renderer,
    // or a GLES route for what it cannot take; it is NOT re-widening this.
    type Dispatch = compositor_support_smithay_dispatch_state_base::state::Dispatch;
    let handle = _loop.state.output.display_handle.clone();
    let node = _loop.inner.kernel
        .get(&compositor_orchestration_core_state_base::state::GPU_BINDING)
        .as_ref()
        .map(|b| b.borrow().primary);
    _loop.state.dmabuf.global = Some(match node {
        Some(scanout) => {
            let main_device: dev_t = registrar::composite_node(scanout).dev_id();
            let feedback = DmabufFeedbackBuilder::new(main_device, dma_formats)
                .build()
                .unwrap_or_else(|e| abort!("Failed to build dmabuf feedback: {e:?}"));
            info!("dmabuf global v5, main_device {main_device:#x}");
            _loop.state.dmabuf.state.create_global_with_default_feedback::<Dispatch>(&handle, &feedback)
        }
        None => {
            warn!("dmabuf global v3 instead of v5: the primary gpu is not set");
            _loop.state.dmabuf.state.create_global::<Dispatch>(&handle, dma_formats)
        }
    });
}

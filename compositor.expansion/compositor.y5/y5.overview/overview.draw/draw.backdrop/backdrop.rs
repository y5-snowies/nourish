//! Freeze-backdrop capture for the overview.
//!
//! `arm` runs in the GLES prepare phase: on open it requests an output-
//! framebuffer capture (the kernel ticks it each frame). Because the overlay is
//! not drawn until the phase is `Ready`, the next couple of frames compose the
//! desktop — which is what fills the capture entry — and then we snapshot it to
//! a frozen `SnapshotHandle`. `snapshot_dmabuf` exposes that frozen frame's
//! dmabuf so the scene can import + draw it (dimmed) behind the grid. If no
//! capture registry is available, the phase resolves to `Ready(None)` and the
//! scene falls back to a plain dim.

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_graphic_capture_registry::{CaptureSource, OutputId};
use compositor_y5_overview_state_base::base::{Backdrop, Phase};

/// Drive the freeze-backdrop capture once per frame (GLES prepare phase).
pub fn arm(state: &mut Loop, gles: &mut GlesRenderer, _size: Size<i32, Physical>) {
    if !state.inner.overview().visible {
        if !matches!(state.inner.overview().phase, Phase::Closed) {
            state.inner.overview_mut().phase = Phase::Closed;
        }
        return;
    }
    if state.inner.overview().overlay_ready() {
        return;
    }

    match &state.inner.overview().phase {
        Phase::Closed => {
            let gpu = state.inner.environment.GPU.clone();
            let output_id = OutputId::from_key(&state.inner.active_output_key());
            let entry = state
                .inner
                .kernel
                .get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY_MUT)
                .as_mut()
                .and_then(|reg| {
                    reg.request(&gpu, gles, CaptureSource::OutputFramebuffer(output_id)).ok()
                });
            state.inner.overview_mut().phase = match entry {
                Some(entry) => Phase::Arming { entry, countdown: 2 },
                None => Phase::Ready(None),
            };
        }
        Phase::Arming { .. } => {
            let done = {
                if let Phase::Arming { countdown, .. } = &mut state.inner.overview_mut().phase {
                    if *countdown > 0 {
                        *countdown -= 1;
                        false
                    } else {
                        true
                    }
                } else {
                    false
                }
            };
            if done {
                let gpu = state.inner.environment.GPU.clone();
                let backdrop = if let Phase::Arming { entry, .. } = &state.inner.overview().phase {
                    match entry.snapshot(&gpu, gles).ok() {
                        // Blur the frozen desktop; fall back to the sharp snapshot
                        // if the blur passes fail.
                        Some(snap) => Some(
                            match compositor_y5_overview_draw_blur::blur::blur(gles, &gpu, &snap) {
                                Some(blurred) => Backdrop::Blur(blurred),
                                None => Backdrop::Sharp(snap),
                            },
                        ),
                        None => None,
                    }
                } else {
                    None
                };
                state.inner.overview_mut().phase = Phase::Ready(backdrop);
            }
        }
        Phase::Ready(_) => {}
    }
}

/// The frozen desktop's (blurred, full-res) dmabuf, once resolved — for the
/// scene to import + draw 1:1.
pub fn snapshot_dmabuf(state: &Loop) -> Option<Dmabuf> {
    match &state.inner.overview().phase {
        Phase::Ready(Some(Backdrop::Blur(a))) => Some(a.dmabuf.clone()),
        Phase::Ready(Some(Backdrop::Sharp(s))) => Some(s.dmabuf().clone()),
        _ => None,
    }
}

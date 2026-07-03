use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Size};

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_graphic_capture_registry::{CaptureSource, OutputId};
use compositor_y5_picker_state_base::base::Arming;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

/// Frame A of the deferred open: request a framebuffer capture of the still-
/// active origin world. The entry fills over this frame's render (the registry
/// ticks it); frame B snapshots it. With no capture registry we just open.
pub fn arm(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    if state.inner.worlds.active_id() == PICKER_WORLD {
        return;
    }
    let origin = state.inner.worlds.active_id();
    let gpu = state.inner.environment.GPU.clone();
    // Capture the ACTIVE monitor's framebuffer (stable EDID-derived id), not always
    // the primary — the picker opens on the monitor the user is on.
    let output_id = OutputId::from_key(&state.inner.active_output_key());
    let capture = state
        .inner
        .kernel
        .get_mut(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY_MUT)
        .as_mut()
        .and_then(|reg| {
            reg.request(&gpu, renderer, CaptureSource::OutputFramebuffer(output_id))
                .ok()
        });
    match capture {
        Some(capture) => {
            state
                .inner
                .worlds
                .get_mut(PICKER_WORLD)
                .storage_mut()
                .get_mut(&PICKER_MUT)
                .arming = Some(Arming { origin, capture, countdown: 3 });
            info!("picker: armed framebuffer capture of world {origin}");
        }
        None => {
            warn!("picker: capture registry unavailable; opening without a fresh thumbnail");
            compositor_y5_picker_interface_base::base::open(state);
            compositor_y5_picker_scene_create::create::create(state, renderer, size);
        }
    }
}

/// Drains the deferred open across frames: let the capture fill for a few frames
/// → snapshot the origin into a thumbnail → wait ONE more frame (so the snapshot
/// lands) → open + build the scene. No-op if nothing is pending.
pub fn finish_arm_and_open(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    // Phase 2: a snapshot was taken last frame — now open + build the scene.
    let pending = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).pending_open.take();
    if pending.is_some() {
        compositor_y5_picker_interface_base::base::open(state);
        compositor_y5_picker_scene_create::create::create(state, renderer, size);
        return;
    }

    // Phase 1: wait out the fill countdown, then snapshot and arm the open.
    let arming_slot = state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT);
    match arming_slot.arming.as_mut() {
        None => return,
        Some(a) if a.countdown > 0 => {
            a.countdown -= 1;
            return;
        }
        Some(_) => {}
    }
    let arming = arming_slot.arming.take().expect("arming present (checked above)");

    let gpu = state.inner.environment.GPU.clone();
    match arming.capture.snapshot(&gpu, renderer) {
        Ok(snapshot) => {
            let snap_size = snapshot.size();
            state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).thumbnails.insert(arming.origin, snapshot);
            info!("picker: thumbnail for world {} ({}x{})", arming.origin, snap_size.w, snap_size.h);
        }
        Err(e) => warn!("picker: thumbnail snapshot for world {} failed: {e:?}", arming.origin),
    }
    // Snapshot taken; open on the NEXT frame so it has time to land.
    state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).pending_open = Some(arming.origin);
}

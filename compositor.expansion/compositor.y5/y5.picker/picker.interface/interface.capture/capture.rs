use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Size};

use compositor_orchestration_core_state_base::Loop;
use compositor_y5_graphic_capture_registry::{CaptureSource, OutputId};
use compositor_y5_picker_state_base::base::{Arming, PickerState};
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};
use compositor_y5_picker_three_constant::FADE_OUT_SECS;

/// The picker world's state slot.
fn picker(state: &mut Loop) -> &mut PickerState {
    state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT)
}

/// Frame A of the deferred open: start the outgoing world's fade to black, and
/// request a framebuffer capture of it (still active, so the thumbnail shows the
/// world rather than the picker). The entry fills over this frame's render (the
/// registry ticks it); a later frame snapshots it. Without a capture registry the
/// open still proceeds — just without a fresh thumbnail.
pub fn arm(state: &mut Loop, renderer: &mut GlesRenderer, _size: Size<i32, Physical>) {
    if state.inner.worlds.active_id() == PICKER_WORLD {
        return;
    }
    let origin = state.inner.worlds.active_id();
    // The transition starts NOW, not when the capture lands — the fill frames are
    // absorbed by the fade-out rather than added in front of it.
    picker(state).fade_out = Some(std::time::Instant::now());
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
            picker(state).arming = Some(Arming { origin, capture, countdown: 3 });
            info!("picker: armed framebuffer capture of world {origin}");
        }
        None => warn!("picker: capture registry unavailable; opening without a fresh thumbnail"),
    }
}

/// Drains the deferred open across frames: let the capture fill for a few frames
/// → snapshot the origin into a thumbnail → keep the origin world composing (under
/// the fade-out overlay) until it is fully black → open + build the scene. No-op
/// if nothing is pending.
pub fn finish_arm_and_open(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    // Phase 1: wait out the fill countdown, then snapshot into the thumbnail.
    let ready = match picker(state).arming.as_mut() {
        Some(a) if a.countdown > 0 => {
            a.countdown -= 1;
            false
        }
        Some(_) => true,
        None => false,
    };
    if ready {
        let arming = picker(state).arming.take().expect("arming present (checked above)");
        let gpu = state.inner.environment.GPU.clone();
        match arming.capture.snapshot(&gpu, renderer) {
            Ok(snapshot) => {
                let snap_size = snapshot.size();
                picker(state).thumbnails.insert(arming.origin, snapshot);
                info!("picker: thumbnail for world {} ({}x{})", arming.origin, snap_size.w, snap_size.h);
            }
            Err(e) => warn!("picker: thumbnail snapshot for world {} failed: {e:?}", arming.origin),
        }
    }

    // Phase 2: switch only once the outgoing world has faded fully out, so the
    // first picker frame lands on black and its own fade-in continues from there.
    // Keep the vblank cycle alive meanwhile — nothing else is animating.
    let (armed, since) = { let p = picker(state); (p.arming.is_some(), p.fade_out) };
    let Some(since) = since else { return };
    state.schedule_redraw_post_vblank();
    if armed || since.elapsed().as_secs_f32() < FADE_OUT_SECS {
        return;
    }
    // `fade_out` is deliberately NOT cleared here — the picker's own pass does
    // that. The kernel samples `picker_active` (and so picks Scene vs Picker) at
    // the TOP of the frame, but this switch runs inside the Scene pass's prepare,
    // so THIS frame still composes through the main scene — now against a world
    // with no windows and no canvas. Leaving the overlay opaque keeps that frame
    // black instead of blinking the empty picker world between the two fades.
    compositor_y5_picker_interface_base::base::open(state);
    compositor_y5_picker_scene_create::create::create(state, renderer, size);
}

//! Drains the rim-issued `DisplayRequest` (set by the lid policy) and performs
//! the kernel-side effect: logind suspend, DPMS power, or active-output switch.
//! Called from the libinput loop source so it runs regardless of render/DPMS
//! state (a lid-open arrives as an input event in the same cycle).

use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use compositor_kernel_scanout_surface_output_base::output;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_driver_lid_base::base::{
    DisplayRequest, DISPLAY_OFF_MUT, DISPLAY_REQUEST_MUT,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Take and perform at most one pending display request.
pub fn drain(state: &mut Loop, ctx_rc: &Rc<RefCell<NativeRenderContext>>) {
    let Some(request) = state.inner.kernel.get_mut(&DISPLAY_REQUEST_MUT).take() else {
        return;
    };

    match request {
        DisplayRequest::Suspend => {
            info!("display request: Suspend (logind)");
            match state
                .inner
                .kernel
                .get(&compositor_orchestration_driver_logind_base::base::LOGIND)
            {
                Some(logind) => logind.suspend(),
                None => warn!("Suspend requested but logind is unavailable"),
            }
        }
        DisplayRequest::PanelOff => panel_dpms(state, ctx_rc, false),
        DisplayRequest::PanelOn => panel_dpms(state, ctx_rc, true),
        // Docked lid-close / undock. Moving the active scanout to a different
        // connector requires re-running the renderer-bound pipe bring-up
        // (initialize_output with the GLES element types, a fresh CRTC/mode, and
        // a rebuilt damage tracker) — see `native.assemble/assemble.renderer`.
        // That path must be validated on real docked hardware; until it lands we
        // SAFELY DEGRADE by leaving the internal panel active (never blanking the
        // only proven display), so lid-close-while-docked is a no-op rather than
        // a black screen. The switch helper plugs in here.
        DisplayRequest::SwitchToExternal => {
            warn!(
                "SwitchToExternal: keeping internal panel active; external scanout \
                 hand-off needs the hardware-validated pipe bring-up (not yet wired)"
            );
        }
        DisplayRequest::SwitchToInternal => {
            warn!("SwitchToInternal: internal panel already active (no-op)");
        }
    }
}

/// Power the active connector's display on/off via DPMS, gating the frame
/// executor in tandem (a page-flip would re-power a blanked connector).
fn panel_dpms(state: &mut Loop, ctx_rc: &Rc<RefCell<NativeRenderContext>>, on: bool) {
    info!("display request: panel DPMS {}", if on { "on" } else { "off" });
    let mut ctx = ctx_rc.borrow_mut();
    let ctx_ref = &mut *ctx;

    if on {
        // Power on first, then a forced modeset + surface/buffer reset to repaint
        // (same recovery the session-resume path performs).
        if let Some(o) = ctx_ref.pipe().drm_output.as_ref() {
            if let Err(e) = output::set_dpms(o, true) {
                warn!("DPMS on failed: {e}");
            }
        }
        if let Err(e) = output::activate(&mut ctx_ref.drm_output_manager.borrow_mut(), true) {
            warn!("DPMS-on modeset failed: {e}");
        }
        if let Some(o) = ctx_ref.pipe_mut().drm_output.as_mut() {
            if let Err(e) = output::reset(o) {
                warn!("DPMS-on surface reset failed: {e}");
            }
        }
        drop(ctx);
        // Ungate the executor and kick a repaint.
        *state.inner.kernel.get_mut(&DISPLAY_OFF_MUT) = false;
        state.schedule_redraw();
    } else {
        // Gate the executor before blanking so no stray flip re-powers it.
        *state.inner.kernel.get_mut(&DISPLAY_OFF_MUT) = true;
        if let Some(o) = ctx_ref.pipe().drm_output.as_ref() {
            if let Err(e) = output::set_dpms(o, false) {
                warn!("DPMS off failed: {e}");
            }
        }
    }
}

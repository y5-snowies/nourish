//! Live output-mode changes from the settings window with a user-confirmed fault
//! gate: a provisional apply the user must KEEP within a timeout or it
//! auto-reverts. Driven via OUTPUT_MODE_REQUEST (the rim can't reach the DRM
//! output); drained from the kernel input loop, like the lid.
use compositor_kernel_gles_element_wrap_base::wrap::GlesElementWrapper;
use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use compositor_orchestration_core_state_base::state::StateDRMBinding;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_scene_element::element::SceneElement;
use compositor_orchestration_driver_output_base::base::{ApplyResult, ModeInfo, OutputModeRequest, OUTPUTS_SNAPSHOT_MUT, OUTPUT_MODES_SNAPSHOT, OUTPUT_MODES_SNAPSHOT_MUT, OUTPUT_MODE_REQUEST_MUT, OUTPUT_MODE_RESULT_MUT};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::drm::control::Mode as DrmMode;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
/// How long a provisionally-applied mode survives without an explicit Keep.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);
type Ctx = Rc<RefCell<NativeRenderContext>>;
fn set_result(state: &mut Loop, r: ApplyResult) {
    *state.inner.kernel.get_mut(&OUTPUT_MODE_RESULT_MUT) = Some(r);
}
fn mode_info(m: DrmMode) -> ModeInfo {
    ModeInfo { width: m.size().0, height: m.size().1, refresh_mhz: m.vrefresh() * 1000 }
}
/// After a CONFIRMED mode change, refresh the changed monitor's `current` in the
/// rim-facing snapshots (matched by `edid_key`, NOT the primary) so reopening the
/// settings window shows the new mode. A mode change leaves the connector list and
/// each monitor's advertised modes unchanged, so only `current` needs updating.
fn refresh_snapshot_current(state: &mut Loop, edid_key: &str, cur: ModeInfo) {
    if state.inner.kernel.get(&OUTPUT_MODES_SNAPSHOT).edid_key == edid_key {
        state.inner.kernel.get_mut(&OUTPUT_MODES_SNAPSHOT_MUT).current = Some(cur);
    }
    if let Some(d) = state.inner.kernel.get_mut(&OUTPUTS_SNAPSHOT_MUT).displays.iter_mut().find(|d| d.edid_key == edid_key) {
        d.current = Some(cur);
    }
}
/// Index of the pipe driving the output identified by `edid_key`, or `None`.
fn pipe_of(ctx: &NativeRenderContext, edid_key: &str) -> Option<usize> {
    ctx.outputs.iter().position(|p| {
        compositor_orchestration_core_state_base::state::output_key(&p.output) == edid_key
    })
}
/// Apply `mode` to the pipe at `idx` and propagate to its smithay Output.
fn set_mode_now(ctx: &mut NativeRenderContext, idx: usize, mode: DrmMode) -> Result<(), String> {
    let gpu = ctx.gpu_binding.clone();
    let mut binding = gpu.borrow_mut();
    let StateDRMBinding { gpus, primary } = &mut *binding;
    let mut renderer = gpus.single_renderer(primary).map_err(|e| format!("renderer: {e:?}"))?;
    let output = ctx.outputs[idx].drm_output.as_mut().ok_or_else(|| "output not live".to_string())?;
    compositor_kernel_scanout_surface_reconfigure_base::reconfigure::set_output_mode::<_, GlesElementWrapper<SceneElement<GlesRenderer>>>(output, mode, &mut renderer)?;
    drop(binding);
    let m = smithay::output::Mode::from(mode);
    ctx.outputs[idx].mode = m;
    ctx.outputs[idx].current_drm_mode = mode;
    ctx.outputs[idx].output.change_current_state(Some(m), None, None, None);
    Ok(())
}
/// Take a pending request (if any) and DEFER it onto a one-shot loop timer, so the
/// modeset never runs inside the vblank/render callback that may be calling this
/// drain (a modeset mid-vblank is unsafe — same rule as `display.reconcile`).
pub fn drain(state: &mut Loop, ctx_rc: &Ctx) {
    let Some(req) = state.inner.kernel.get_mut(&OUTPUT_MODE_REQUEST_MUT).take() else { return };
    let ctx = ctx_rc.clone();
    state
        .loop_handle
        .insert_source(Timer::immediate(), move |_, _, state: &mut Loop| {
            match &req {
                OutputModeRequest::Apply { edid_key, width, height, refresh_mhz } => apply(state, &ctx, edid_key, *width, *height, *refresh_mhz),
                OutputModeRequest::Confirm => finish(state, &ctx, false),
                OutputModeRequest::Revert => finish(state, &ctx, true),
            }
            TimeoutAction::Drop
        })
        .expect("output mode deferral timer registration failed");
}
fn apply(state: &mut Loop, ctx_rc: &Ctx, edid_key: &str, w: u16, h: u16, mhz: u32) {
    let mut ctx = ctx_rc.borrow_mut();
    // Resolve the SELECTED monitor's pipe (multi-output) — the mode is matched
    // against THAT pipe's advertised modes, and applied to THAT pipe.
    let Some(idx) = pipe_of(&ctx, edid_key) else {
        drop(ctx);
        warn!("mode apply: no live pipe for {edid_key}");
        return set_result(state, ApplyResult::Failed);
    };
    let Some(target) = ctx.outputs[idx].modes.iter().copied().find(|m| m.size() == (w, h) && m.vrefresh() * 1000 == mhz) else {
        drop(ctx);
        warn!("requested mode {w}x{h} not advertised on {edid_key}");
        return set_result(state, ApplyResult::Failed);
    };
    // Keep the ORIGINAL baseline across re-applies; cancel any pending timer.
    let baseline = match ctx.outputs[idx].mode_revert.take() {
        Some((prev, token)) => { state.loop_handle.remove(token); prev }
        None => ctx.outputs[idx].current_drm_mode,
    };
    if let Err(e) = set_mode_now(&mut ctx, idx, target) {
        warn!("live mode apply failed: {e}");
        let _ = set_mode_now(&mut ctx, idx, baseline);
        drop(ctx);
        return set_result(state, ApplyResult::Failed);
    }
    drop(ctx);
    state.schedule_redraw();
    // Arm the per-pipe confirm/revert watchdog. `idx` is still valid — no hotplug can
    // interleave this synchronous drain; the watchdog itself re-resolves by key.
    let token = arm(state, ctx_rc, edid_key.to_string(), baseline);
    ctx_rc.borrow_mut().outputs[idx].mode_revert = Some((baseline, token));
    set_result(state, ApplyResult::Provisional);
}
/// `revert=false` keeps a pending mode (Confirm); `true` restores baseline. The
/// target pipe is the one with an armed `mode_revert` (only one at a time).
fn finish(state: &mut Loop, ctx_rc: &Ctx, revert: bool) {
    let mut ctx = ctx_rc.borrow_mut();
    let Some(idx) = ctx.outputs.iter().position(|p| p.mode_revert.is_some()) else { return };
    let (baseline, token) = ctx.outputs[idx].mode_revert.take().expect("checked is_some");
    state.loop_handle.remove(token);
    if revert {
        if let Err(e) = set_mode_now(&mut ctx, idx, baseline) { warn!("revert failed: {e}"); }
        drop(ctx);
        state.schedule_redraw();
        set_result(state, ApplyResult::Reverted);
    } else {
        let cur = mode_info(ctx.outputs[idx].current_drm_mode);
        let edid = compositor_orchestration_core_state_base::state::output_key(&ctx.outputs[idx].output);
        drop(ctx);
        refresh_snapshot_current(state, &edid, cur);
        set_result(state, ApplyResult::Confirmed);
    }
}
fn arm(state: &mut Loop, ctx_rc: &Ctx, edid_key: String, baseline: DrmMode) -> RegistrationToken {
    let ctx = ctx_rc.clone();
    state.loop_handle.insert_source(Timer::from_duration(CONFIRM_TIMEOUT), move |_, _, state: &mut Loop| {
        let mut c = ctx.borrow_mut();
        // Re-resolve the target pipe by key (hotplug may have reordered `outputs`).
        if let Some(idx) = pipe_of(&c, &edid_key) {
            if c.outputs[idx].mode_revert.take().is_some() {
                if let Err(e) = set_mode_now(&mut c, idx, baseline) { warn!("auto-revert failed: {e}"); }
                drop(c);
                state.schedule_redraw();
                set_result(state, ApplyResult::Reverted);
            }
        }
        TimeoutAction::Drop
    }).expect("mode revert watchdog registration failed")
}

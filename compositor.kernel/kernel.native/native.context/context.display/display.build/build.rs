//! Build a `NativeDrmOutput` for an arbitrary connected connector at runtime,
//! REUSING the existing smithay `Output`, for the live monitor switch. The active
//! output keeps its own CRTC (passed as `busy`) so a revert is a clean swap-back
//! to it. Mirrors the assembly pipe bring-up (`assemble.renderer`) for one
//! additional connector, over the same validating mode fallback chain.

use compositor_kernel_drm_edid_parse_base::parse::HdrInfo;
use compositor_kernel_gles_element_wrap_base::wrap::GlesElementWrapper;
use compositor_kernel_scanout_surface_output_base::output::{NativeDrmOutput, NativeDrmOutputManager};
use compositor_orchestration_core_state_base::state::StateDRMBinding;
use compositor_orchestration_draw_scene_element::element::SceneElement;
use compositor_orchestration_driver_output_base::base::ModeInfo;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::{Mode, Output};
use smithay::reexports::drm::control::{connector, crtc, Mode as DrmMode};
use std::cell::RefCell;
use std::rc::Rc;

/// The runtime-built pipe plus the metadata the switch gate swaps into the context.
pub struct BuiltOutput {
    pub drm_output: NativeDrmOutput,
    /// The CRTC this pipe was claimed on (routes VBlanks; identifies the pipe).
    pub crtc: crtc::Handle,
    pub drm_mode: DrmMode,
    pub modes: Vec<DrmMode>,
    pub connector: connector::Handle,
    pub hdr: HdrInfo,
}

/// The mode to bring up: the requested advertised mode if it matches, else the
/// default policy. Errors only if the connector advertises nothing.
fn pick_mode(target: &connector::Info, requested: Option<ModeInfo>) -> Result<DrmMode, String> {
    if let Some(r) = requested {
        if let Some(m) = target.modes().iter().find(|m| {
            let (w, h) = m.size();
            w == r.width && h == r.height && m.vrefresh() * 1000 == r.refresh_mhz
        }) {
            return Ok(*m);
        }
    }
    compositor_kernel_drm_mode_select_base::select::select_default(target)
        .ok_or_else(|| "connector advertises no modes".to_string())
}

/// Bring up a second pipe for `target` on a free CRTC, reusing `output`. Returns
/// `Err` (leaving the active pipe untouched) on no free CRTC / no mode / modeset
/// failure, so the caller can report a clean `Failed` with no state change.
pub fn build(
    manager: &Rc<RefCell<NativeDrmOutputManager>>,
    gpu_binding: &Rc<RefCell<StateDRMBinding>>,
    output: &Output,
    busy: &[crtc::Handle],
    target: &connector::Info,
    requested: Option<ModeInfo>,
) -> Result<BuiltOutput, String> {
    let mut binding = gpu_binding.borrow_mut();
    let StateDRMBinding { gpus, primary } = &mut *binding;
    let mut renderer = gpus.single_renderer(primary).map_err(|e| format!("renderer: {e:?}"))?;

    let mut mgr = manager.borrow_mut();
    // Read everything off the device before the mutable `initialize` borrow.
    let (pipe, hdr) = {
        let drm = mgr.device();
        let res = compositor_kernel_drm_connector_scan_base::scan::resources(drm);
        let pipe =
            compositor_kernel_scanout_pipe_claim_free::free::claim_excluding(drm, target, &res, busy)
                .ok_or_else(|| "no free CRTC for switch".to_string())?;
        let hdr = compositor_kernel_drm_edid_parse_base::parse::read(drm, target)
            .as_ref()
            .map(compositor_kernel_drm_edid_parse_base::parse::parse_hdr)
            .unwrap_or_default();
        (pipe, hdr)
    };

    // Validating modeset over the fallback chain (selected mode first). Each
    // candidate is TEST-committed by `initialize_output`; the first that passes
    // drives the pipe, chain exhaustion is the error. With drop-first switching the
    // old output is already gone (the manager's compositor map is empty), so this
    // is a plain single-output bring-up like startup.
    let selected = pick_mode(target, requested)?;
    info!(
        "output switch build: connector {:?} crtc {pipe:?} preferred {}x{}@{}",
        target.handle(),
        selected.size().0,
        selected.size().1,
        selected.vrefresh()
    );
    let chain = compositor_kernel_scanout_commit_test_base::test::fallback_chain(target, selected);
    let mut slot: Option<NativeDrmOutput> = None;
    let chosen = compositor_kernel_scanout_commit_test_base::test::try_chain(chain, |mode| {
        // The reused smithay `Output` still carries the OLD mode; `initialize_output`
        // sizes the primary plane from it, so it must match THIS candidate or the
        // plane won't fit the CRTC mode and the atomic test fails with EINVAL.
        output.change_current_state(Some(Mode::from(mode)), None, None, None);
        match compositor_kernel_scanout_surface_output_base::output::initialize::<
            _,
            GlesElementWrapper<SceneElement<GlesRenderer>>,
        >(&mut mgr, pipe, mode, &[target.handle()], output, &mut renderer)
        {
            Ok(out) => {
                slot = Some(out);
                Ok(())
            }
            Err(e) => Err(e),
        }
    })?;

    Ok(BuiltOutput {
        drm_output: slot.ok_or_else(|| "try_chain ok without output".to_string())?,
        crtc: pipe,
        drm_mode: chosen,
        modes: target.modes().to_vec(),
        connector: target.handle(),
        hdr,
    })
}

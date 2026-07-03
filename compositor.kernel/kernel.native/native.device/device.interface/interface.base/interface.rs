//! The explicit-settings surface of the device authority — the integration
//! API the main project calls against the handles `wire.entry::wire` returns.
//! Typed setting application routed to the owning mechanisms; mode changes
//! flow HERE and nowhere else.
//!
//! Failure policy: a runtime setting that fails to apply is not
//! self-recovering — panic.

use compositor_kernel_gles_element_wrap_base::wrap::GlesElementWrapper;
use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use compositor_kernel_graphic_preference_enable_safety::safety::SafetyEnable;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::reexports::drm::control::Mode as DrmMode;
use compositor_orchestration_draw_scene_element::element::SceneElement;
use compositor_orchestration_core_state_base::state::StateDRMBinding;

#[derive(Debug)]
pub enum DeviceSetting {
    /// Set a (validated or synthesized) mode on the running pipe.
    Mode(DrmMode),
    /// Update the live Law-7 safety-net enablement set.
    Safety(SafetyEnable),
}

/// Apply one setting to the running device. Mode application is real
/// (delegated through `scanout.surface/surface.reconfigure`) and propagates
/// to the smithay Output so clients observe the change.
pub fn apply(ctx: &mut NativeRenderContext, setting: DeviceSetting) {
    match setting {
        DeviceSetting::Mode(drm_mode) => {
            let gpu_binding = ctx.gpu_binding.clone();
            let mut binding = gpu_binding.borrow_mut();
            let StateDRMBinding { gpus, primary } = &mut *binding;
            let mut renderer = gpus
                .single_renderer(primary)
                .expect("renderer unavailable for mode application");

            let output = ctx
                .pipe_mut()
                .drm_output
                .as_mut()
                .unwrap_or_else(|| abort!("mode application with no active output"));
            compositor_kernel_scanout_surface_reconfigure_base::reconfigure::set_output_mode::<
                _,
                GlesElementWrapper<SceneElement<GlesRenderer>>,
            >(output, drm_mode, &mut renderer)
            .unwrap_or_else(|e| abort!("mode application failed: {e}"));

            // Propagate to the smithay Output so clients observe the change.
            let mode = smithay::output::Mode::from(drm_mode);
            ctx.pipe_mut().mode = mode;
            ctx.pipe().output.change_current_state(Some(mode), None, None, None);
            info!(
                "device.interface: mode applied {}x{}@{}",
                drm_mode.size().0,
                drm_mode.size().1,
                drm_mode.vrefresh()
            );
        }
        DeviceSetting::Safety(enable) => {
            ctx.safety = enable;
            info!("device.interface: safety enables updated: {enable:?}");
        }
    }
}

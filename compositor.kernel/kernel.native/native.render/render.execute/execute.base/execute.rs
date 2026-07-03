//! The frame executor: runs the compositor-issued FramePlan against the
//! hosted pipe. (Ex draw.scene/scene.rs `scene()`, now plan-driven — the pass
//! presence/ordering comes from `compositor_kernel_graphic_draw_plan_frame`, not from a
//! local Status match. Pass KINDS are compositor vocabulary; this crate maps
//! each kind to its element source.)
//!
//! The Rc<RefCell<renderer>> borrow choreography is carried verbatim from the
//! original, including its documented reasoning about the bind+blit
//! double-borrow problem.
//!
//! Completion-pass semantics:
//! - frame flags come from the plane policy (`scanout.plane/plane.direct`),
//!   not a hardcoded DEFAULT;
//! - the post-scene tap fires only when the PLAN places it AND a subscriber
//!   is active (`ctx.tap_subscriptions`), which is also when the capture
//!   registry is consulted;
//! - queue failures panic outside the session-resume window (see
//!   `scanout.flip/flip.queue`);
//! - the executor reports a `FrameOutcome` so the pacing layer (`wire.frame`)
//!   can act on empty frames when the `flip-estimate` net is compiled in and
//!   enabled.

use compositor_kernel_gles_element_wrap_base::wrap::GlesElementWrapper;
use compositor_kernel_gles_element_combined_base::combined::OutputElement;
use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use compositor_kernel_vulkan_renderer_core_base::renderer::VulkanRenderer;
use compositor_kernel_graphic_draw_plan_frame::frame::{plan, FramePass};
use compositor_kernel_graphic_draw_plan_tap::tap::POST_SCENE;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{Bind, RendererSuper};
use smithay::reexports::calloop::LoopHandle;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};
use std::cell::RefCell;
use std::rc::Rc;
use compositor_orchestration_core_state_base::state::{StateDRMBinding, StatusSession};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_graphic_capture_registry::{CaptureRegistry, OutputId};

type VkScene = compositor_orchestration_draw_scene_element::element::SceneElement<VulkanRenderer>;
type VkLock = compositor_y5_lock_scene_element::element::LockSceneElement<VulkanRenderer>;

/// Honor `RenderFrameResult::needs_sync()` before queueing to KMS: when smithay
/// can't hand the atomic commit a GPU fence (device lacks fencing, or the
/// render's SyncPoint isn't an exportable fd), it is *our* responsibility to
/// CPU-wait for render completion before `queue_frame`, or KMS may scan out a
/// half-rendered buffer. When fencing IS available (`needs_sync()==false`),
/// this is a no-op and smithay attaches our fence as the commit IN_FENCE — the
/// best (no-CPU-wait) path. Cheap insurance that keeps every renderer correct.
fn honor_needs_sync<B, F, E>(
    result: &smithay::backend::drm::compositor::RenderFrameResult<'_, B, F, E>,
) where
    B: smithay::backend::allocator::Buffer,
    F: smithay::backend::drm::Framebuffer,
{
    use smithay::backend::drm::compositor::PrimaryPlaneElement;
    if result.needs_sync() {
        if let PrimaryPlaneElement::Swapchain(ref element) = result.primary_element {
            if let Err(err) = element.sync.wait() {
                warn!("native: render fence wait interrupted before queue_frame: {err:?}");
            }
        }
    }
}

/// Combined scanout element for the native Vulkan path: one render_frame list
/// carrying both scene and lock elements (lock is placed in front). Delegates
/// everything to the inner `SceneElement`/`LockSceneElement<VulkanRenderer>`.
enum VkOutput {
    Scene(VkScene),
    Lock(VkLock),
}

impl Element for VkOutput {
    fn id(&self) -> &Id {
        match self { Self::Scene(e) => e.id(), Self::Lock(e) => e.id() }
    }
    fn current_commit(&self) -> CommitCounter {
        match self { Self::Scene(e) => e.current_commit(), Self::Lock(e) => e.current_commit() }
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        match self { Self::Scene(e) => e.src(), Self::Lock(e) => e.src() }
    }
    fn geometry(&self, s: Scale<f64>) -> Rectangle<i32, Physical> {
        match self { Self::Scene(e) => e.geometry(s), Self::Lock(e) => e.geometry(s) }
    }
    fn location(&self, s: Scale<f64>) -> Point<i32, Physical> {
        match self { Self::Scene(e) => e.location(s), Self::Lock(e) => e.location(s) }
    }
    fn transform(&self) -> Transform {
        match self { Self::Scene(e) => e.transform(), Self::Lock(e) => e.transform() }
    }
    fn damage_since(&self, s: Scale<f64>, c: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        match self { Self::Scene(e) => e.damage_since(s, c), Self::Lock(e) => e.damage_since(s, c) }
    }
    fn opaque_regions(&self, s: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        match self { Self::Scene(e) => e.opaque_regions(s), Self::Lock(e) => e.opaque_regions(s) }
    }
    fn alpha(&self) -> f32 {
        match self { Self::Scene(e) => e.alpha(), Self::Lock(e) => e.alpha() }
    }
    fn kind(&self) -> Kind {
        match self { Self::Scene(e) => e.kind(), Self::Lock(e) => e.kind() }
    }
}

impl RenderElement<VulkanRenderer> for VkOutput {
    fn draw(
        &self,
        frame: &mut <VulkanRenderer as RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), <VulkanRenderer as RendererSuper>::Error> {
        match self {
            Self::Scene(e) => e.draw(frame, src, dst, damage, opaque, cache),
            Self::Lock(e) => e.draw(frame, src, dst, damage, opaque, cache),
        }
    }
    fn underlying_storage(&self, r: &mut VulkanRenderer) -> Option<UnderlyingStorage<'_>> {
        match self {
            Self::Scene(e) => e.underlying_storage(r),
            Self::Lock(e) => e.underlying_storage(r),
        }
    }
}

/// What this execute() call did, for the pacing layer.
#[derive(Debug)]
pub enum FrameOutcome {
    /// A frame was rendered and queued; a VBlank will follow.
    Queued,
    /// Nothing was queued (no damage, empty plan, paused, or the queue was
    /// deferred to the resume watchdog); frame callbacks already handled.
    Idle,
    /// Empty damage and the estimate-pacing net is active: NO frame
    /// callbacks were sent — `wire.frame` delivers them at the estimated
    /// next vblank.
    #[cfg(feature = "flip-estimate")]
    EmptyDeferred {
        output: smithay::output::Output,
        visible: Vec<smithay::desktop::Window>,
    },
}

/// Which outputs this `execute()` call may render. The per-vblank path passes
/// `Crtc(handle)` so ONLY the output that just flipped is re-rendered — it is
/// structurally impossible to produce a frame for a monitor that has not
/// vblanked, which is what decouples each monitor's refresh cadence. The ping /
/// kickstart / resume-watchdog paths pass `All` to (re)start every idle output.
#[derive(Debug, Clone, Copy)]
pub enum RenderScope {
    /// Every output that is idle (not mid-flip) — ping, kickstart, resume.
    All,
    /// Only the output whose CRTC just delivered a VBlank — per-monitor pacing.
    Crtc(smithay::reexports::drm::control::crtc::Handle),
}

pub fn execute(
    ctx_rc: Rc<RefCell<NativeRenderContext>>,
    loop_handle: LoopHandle<'static, Loop>,
    state: &mut Loop,
    scope: RenderScope,
) -> FrameOutcome {
    let _ = &loop_handle; // retained for parity with the original signature
    if let StatusSession::Paused = state.inner.status_session {
        return FrameOutcome::Idle;
    }
    // DPMS-off gate: a page-flip would re-power the blanked connector, so skip
    // frame production entirely while the panel is powered down (lid/idle).
    if *state.inner.kernel.get(&compositor_orchestration_driver_lid_base::base::DISPLAY_OFF) {
        return FrameOutcome::Idle;
    }

    // Drain any pending output-mode / output-switch transaction from the settings
    // window every render frame, so a provisional Apply and especially a Confirm/
    // Revert take effect promptly instead of waiting for the next libinput event
    // (the request channels are otherwise only drained on input — a still pointer
    // after clicking Keep would let the ~15s watchdog auto-revert). Runs before the
    // context borrow below; both are no-ops when no request is pending.
    compositor_kernel_native_context_display_mode::mode::drain(state, &ctx_rc);
    compositor_kernel_native_context_display_reconcile::reconcile::drain_reconcile(state, &ctx_rc);

    let mut ctx = ctx_rc.borrow_mut();
    let ctx_ref = &mut *ctx;
    // Skip the whole frame only if NO output is live (all in the transient monitor-
    // switch teardown window). Otherwise the per-output loop below skips just the
    // dark ones; every `outputs[idx].drm_output.as_*().unwrap()` is guarded per pipe.
    if ctx_ref.outputs.iter().all(|p| p.drm_output.is_none()) {
        return FrameOutcome::Idle;
    }
    let frame_flags = compositor_kernel_scanout_plane_direct_base::direct::flags();

    let gpu_binding = ctx_ref.gpu_binding.clone();
    let mut binding = gpu_binding.borrow_mut();
    let StateDRMBinding { gpus, primary } = &mut *binding;

    // The capture registry is pre-created at startup (loader prewarm) from the
    // shared bevy context — never built mid-render. Its tap subscription, by
    // contrast, lives on this backend's render context (created during render),
    // so subscribe exactly once here: registry presence IS the tap (Law 5).
    if state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY).is_some()
        && !ctx_ref.tap_subscriptions.is_active(POST_SCENE)
    {
        ctx_ref.tap_subscriptions.subscribe(POST_SCENE);
    }

    // Wrap the renderer in Rc<RefCell> so capture closures can defer borrow
    // tracking to runtime, sidestepping the bind+blit double-borrow problem
    // at compile time.
    let gles_renderer = Rc::new(RefCell::new(gpus.single_renderer(primary).unwrap()));

    // ---- Per-output render loop -------------------------------------------------
    // The renderer + GPU binding above are shared (built once); `size`, the render
    // target, the scene and the page-flip are per output. Each lit CRTC is drawn and
    // flipped in turn on the one GLES renderer. Single-output = one iteration, so the
    // behaviour is unchanged. (Body left at its original indent for review clarity.)
    let mut any_queued = false;
    #[cfg(feature = "flip-estimate")]
    let mut deferred: Option<FrameOutcome> = None;
    // One-time diagnostic: the actual multi-output set the render loop sees.
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static LOGGED: AtomicBool = AtomicBool::new(false);
        if !LOGGED.swap(true, Ordering::Relaxed) {
            let zoom = state.inner.camera().transform.zoom;
            let cam = state.inner.camera().transform.position;
            let lw: Vec<f64> = ctx_ref
                .outputs
                .iter()
                .map(|p| {
                    let s = p.output.current_scale().fractional_scale();
                    p.mode.size.w as f64 / if s.abs() < 1e-6 { 1.0 } else { s }
                })
                .collect();
            let total: f64 = lw.iter().sum();
            info!(
                "MULTI-OUTPUT render: {} pipe(s), camera pos=({:.1},{:.1}) zoom={:.3} layout_total_w={:.0}",
                ctx_ref.outputs.len(),
                cam.x,
                cam.y,
                zoom,
                total,
            );
            for (i, p) in ctx_ref.outputs.iter().enumerate() {
                let props = p.output.physical_properties();
                let scale = p.output.current_scale().fractional_scale();
                let x_left: f64 = lw[..i].iter().sum();
                let center = x_left + lw[i] / 2.0;
                let off_x = (center - total / 2.0) / if zoom.abs() < 1e-6 { 1.0 } else { zoom };
                let geo = state.inner.space_state().state.output_geometry(&p.output);
                info!(
                    "  pipe[{}] crtc={:?} name={:?} edid={:?} mode={}x{} scale={:.2} live={} → render_offset_x={:.1} space_geometry={:?}",
                    i,
                    p.crtc,
                    p.output.name(),
                    format!("{} {} {}", props.make, props.model, props.serial_number),
                    p.mode.size.w,
                    p.mode.size.h,
                    scale,
                    p.drm_output.is_some(),
                    off_x,
                    geo,
                );
            }
        }
    }
    for output_idx in 0..ctx_ref.outputs.len() {
        if ctx_ref.outputs[output_idx].drm_output.is_none() {
            continue;
        }
        // Per-monitor pacing: on a vblank, render ONLY the pipe whose CRTC flipped.
        // Any other output is driven by its OWN vblank — rendering it here would
        // couple its cadence to this one. (All = ping/kickstart/resume: render every
        // idle output.)
        if let RenderScope::Crtc(target) = scope {
            if ctx_ref.outputs[output_idx].crtc != target {
                continue;
            }
        }
        // Skip a pipe whose page-flip is still in flight: its `queued_frame` slot
        // is occupied and won't scan out until its own vblank. Re-rendering it now
        // (on some OTHER output's vblank) would only overwrite that pending frame
        // and burn a CPU render+sync — the coupling that dragged a high-refresh
        // output down to a slower neighbour's rate. Its own vblank clears this and
        // re-renders it. (Single output: its vblank clears it each frame → no skip.)
        if ctx_ref.outputs[output_idx].in_flight {
            continue;
        }
        // Multi-output: force a FULL redraw of this output this frame by resetting
        // the swapchain buffer ages (age 0 ⇒ the OutputDamageTracker clears the whole
        // target and submits ALL elements, instead of the partial/aged-buffer path).
        // The renderers' partial-damage handling isn't correct once each output is
        // paced on its OWN vblank: the aged buffer the tracker assumes still holds the
        // undamaged remainder is stale, so skipped elements vanish — Vulkan clears the
        // whole attachment (heavy blink), GLES shows milder clearing. A full frame is
        // always correct and matches the pre-multi-output behaviour (the fullscreen
        // animating parallax forced near-full damage anyway). Single output keeps the
        // damage optimisation — its buffer age is valid every frame.
        if ctx_ref.outputs.len() > 1 {
            if let Some(o) = ctx_ref.outputs[output_idx].drm_output.as_ref() {
                o.with_compositor(|c| c.reset_buffer_ages());
            }
        }
        let size = ctx_ref.outputs[output_idx].mode.size;
        // Tell the rim which physical output this frame draws, so the focus/
        // coordinate accessors (`current_output()`) resolve THIS output's mode
        // size/scale. Cleared after the loop so the input path falls back to the
        // cursor's output.
        let output_key =
            compositor_orchestration_core_state_base::state::output_key(&ctx_ref.outputs[output_idx].output);
        // Stable capture id for THIS monitor (EDID-derived, not the vec index) so
        // capture entries key the same way the rim's capture requests resolve them.
        let output_id = OutputId::from_key(&output_key);
        state.inner.render_output = Some(output_key.clone());
        // Ensure THIS output has its own view tree (own camera + panes) so the focus
        // accessors resolve THIS monitor's independent camera while drawing — each
        // screen is its own viewport. Use `ensure` (NOT `set_current`): the render
        // loop must not move `current` off the cursor's output (the input systems
        // read `current`); `render_output` above already drives the draw accessors.
        state.inner.output_views_mut().ensure(&output_key);

    // ---- set_output_size: scoped borrow_mut ----
    if let Some(registry) = &state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY) {
        let mut r = gles_renderer.borrow_mut();
        let _ = registry.set_output_size(
            &state.inner.environment.GPU.as_str(),
            r.as_mut(),
            output_id,
            size,
        );
        drop(r);
    }

    // The compositor decides what this frame contains (Law 5): the plan
    // places the tap; the subscription set says whether anyone is listening.
    let picker_active =
        state.inner.worlds.active_id() == compositor_y5_picker_system_base::base::PICKER_WORLD;
    let frame_plan = plan(&state.inner.status, picker_active);
    let render_scene = frame_plan.has_pass(FramePass::Scene);
    let render_lock = frame_plan.has_pass(FramePass::Lock);
    let render_picker = frame_plan.has_pass(FramePass::Picker);
    let tap_post_scene =
        frame_plan.has_tap(POST_SCENE) && ctx_ref.tap_subscriptions.is_active(POST_SCENE);

    // HDR output signalling (M5): apply BT.2020 + PQ to the connector exactly
    // once, after smithay's first modeset has bound the connector (gated on a
    // seen vblank so the prop-only atomic commit references an active connector).
    // A TEST commit validates first; on rejection we fall back to SDR and never
    // retry — a bad blob cannot blank the display.
    if ctx_ref.outputs[output_idx].hdr_active && !ctx_ref.outputs[output_idx].hdr_signalled && (*state.inner.kernel.get(&compositor_orchestration_driver_resume_base::base::VBLANK_SEEN)) {
        match crate::hdr::signal_hdr(&ctx_ref.drm_fd, ctx_ref.outputs[output_idx].connector, &ctx_ref.outputs[output_idx].hdr_caps) {
            Ok(()) => {
                ctx_ref.outputs[output_idx].hdr_signalled = true;
                info!("HDR output signalling applied (connector BT.2020 RGB + PQ metadata)");
            }
            Err(e) => {
                warn!("HDR output signalling failed ({e}); reverting this session to SDR");
                ctx_ref.outputs[output_idx].hdr_active = false;
                ctx_ref.outputs[output_idx].hdr_signalled = true; // don't retry every frame
                let c = &ctx_ref.outputs[output_idx].hdr_caps;
                compositor_developer_stats_registry_base::base::set_hdr_info(
                    false,
                    c.hdr_capable(),
                    "SDR",
                    c.hdr.max_luminance.unwrap_or(0.0),
                    c.colorimetry.bt2020_rgb,
                    "8-bit sRGB",
                );
            }
        }
    }

    let mut last_result_empty = true;
    let mut visible_window: Vec<_> = Vec::new();

    // World-selection screen: the picker overlay owns the frame. Render the bevy
    // sphere-of-cells (prepared on the GLES renderer, then composed by the active
    // renderer) and scan it out. The scene/lock blocks below are no-ops while the
    // picker is active (render_scene/render_lock are false).
    if render_picker {
        // Advance an in-flight video capture: the scene `per_frame` encoder pump
        // doesn't run while the picker owns the frame, so drive it here (the tap
        // below refreshes the capture entry with the picker each frame).
        compositor_y5_graphic_capture_interface::interface::overlay_per_frame(state);
        let picker_clear = [0.04f32, 0.05, 0.10, 1.0];
        let prepared = {
            let mut r = gles_renderer.borrow_mut();
            compositor_y5_picker_scene_frame::frame::prepare(state, r.as_mut(), size)
        };
        if ctx_ref.vulkan_mode {
            let scene = {
                let vk = ctx_ref.vulkan.as_mut().expect("vulkan_mode without renderer");
                compositor_y5_picker_scene_frame::frame::scene::<VulkanRenderer>(
                    state, &mut *vk, size, prepared,
                )
            };
            let outputs: Vec<VkOutput> = scene.Element.into_iter().map(VkOutput::Scene).collect();
            // Post-picker capture tap: keep an in-flight capture recording the
            // world-picker overlay. Screen/full-screen captures blit the composed
            // picker (capture targets set so submit copies it into the entry
            // dmabufs); window/world-region captures render their windows directly.
            let render_job = if tap_post_scene {
                compositor_y5_graphic_capture_interface::render::window_render_job(state)
            } else {
                None
            };
            let targets: Vec<(
                smithay::backend::allocator::dmabuf::Dmabuf,
                Option<Rectangle<i32, Physical>>,
            )> = if tap_post_scene && render_job.is_none() {
                state
                    .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                    .as_ref()
                    .map(|r| r.entry_dmabufs_for_output(output_id))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(_, dmabuf, _, src)| (dmabuf, src))
                    .collect()
            } else {
                Vec::new()
            };
            {
                let vk = ctx_ref.vulkan.as_mut().expect("vulkan_mode without renderer");
                vk.set_capture_targets(targets);
                match ctx_ref
                    .outputs[output_idx]
                    .drm_output
                    .as_mut()
                    .unwrap()
                    .render_frame(&mut *vk, &outputs, picker_clear, frame_flags)
                {
                    Ok(result) => {
                        honor_needs_sync(&result);
                        last_result_empty = result.is_empty;
                    }
                    Err(e) => error!("native vulkan picker render_frame failed: {e:?}"),
                }
            }
            if let Some(job) = render_job {
                let backdrop =
                    compositor_y5_graphic_capture_interface::render::capture_backdrop(state, &job);
                if let Some(mut dmabuf) = state
                    .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                    .as_ref()
                    .and_then(|r| r.entry_dmabuf(job.entry_id))
                {
                    let vk = ctx_ref.vulkan.as_mut().expect("vulkan_mode without renderer");
                    compositor_y5_graphic_capture_interface::render::draw_windows_into_bg(
                        vk,
                        &mut dmabuf,
                        job.size,
                        &job.windows,
                        job.scale,
                        backdrop,
                    );
                }
            }
        } else {
            let scene = {
                let mut r = gles_renderer.borrow_mut();
                compositor_y5_picker_scene_frame::frame::scene::<smithay::backend::renderer::gles::GlesRenderer>(
                    state, r.as_mut(), size, prepared,
                )
            };
            let wrapped: Vec<GlesElementWrapper<_>> =
                scene.Element.iter().map(GlesElementWrapper).collect();
            let mut r = gles_renderer.borrow_mut();
            let picker_result = ctx_ref
                .outputs[output_idx]
                .drm_output
                .as_mut()
                .unwrap()
                .render_frame(&mut *r, &wrapped, picker_clear, frame_flags)
                .unwrap();
            honor_needs_sync(&picker_result);
            last_result_empty = picker_result.is_empty;

            // Post-picker capture tap (GLES): keep an in-flight capture recording
            // the world-picker overlay. Same structure as the scene tap — window/
            // world-region captures render their windows into the entry; screen/
            // full-screen captures blit the composed picker framebuffer.
            if tap_post_scene {
                if let Some(job) =
                    compositor_y5_graphic_capture_interface::render::window_render_job(state)
                {
                    if let Some(mut dmabuf) = state
                        .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                        .as_ref()
                        .and_then(|reg| reg.entry_dmabuf(job.entry_id))
                    {
                        compositor_y5_graphic_capture_interface::render::draw_windows_into(
                            &mut *r,
                            &mut dmabuf,
                            job.size,
                            &job.windows,
                            job.scale,
                        );
                    }
                } else if let Some(registry) = &mut state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY) {
                    let entries = registry.entries_for_output(output_id);
                    let full_src = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), size);

                    for (entry_id, mut entry_tex, entry_size, src_override) in entries {
                        let src = src_override.unwrap_or(full_src);
                        let blit: Result<(), _> = (|| {
                            let mut entry_fb = r.bind(&mut entry_tex).map_err(
                                compositor_y5_graphic_capture_registry::registry::BlitErr::Bind,
                            )?;
                            picker_result
                                .blit_frame_result(
                                    entry_size,
                                    smithay::utils::Transform::Normal,
                                    Scale::from(1.0),
                                    &mut *r,
                                    &mut entry_fb,
                                    [src],
                                    std::iter::empty::<Id>(),
                                )
                                .map(|_sync| ())
                                .map_err(
                                    compositor_y5_graphic_capture_registry::registry::BlitErr::Blit,
                                )
                        })();
                        if let Err(e) = blit {
                            warn!("capture blit failed: entry_id={entry_id:?} err={e:?}");
                        }
                    }
                }
            }

            drop(picker_result);
            drop(r);
        }
    } else if ctx_ref.vulkan_mode {
        // ---- Native Vulkan path: GLES prepare(), then compose + scan out via
        // the VulkanRenderer through the same DrmOutput.
        let mut scene_els: Vec<VkScene> = Vec::new();
        let mut lock_els: Vec<VkLock> = Vec::new();
        if render_scene {
            let prepared = {
                let mut r = gles_renderer.borrow_mut();
                compositor_orchestration_draw_scene_frame::scene::prepare(state, r.as_mut(), size)
            };
            let vk = ctx_ref.vulkan.as_mut().expect("vulkan_mode without renderer");
            let s = compositor_orchestration_draw_scene_frame::scene::scene::<VulkanRenderer>(
                state, vk, size, prepared,
            );
            visible_window = s.visible_window;
            scene_els = s.Element;
        }
        if render_lock {
            let lp = {
                let mut r = gles_renderer.borrow_mut();
                compositor_y5_lock_scene_frame::frame::prepare(state, r.as_mut(), size)
            };
            let vk = ctx_ref.vulkan.as_mut().expect("vulkan_mode without renderer");
            let l = compositor_y5_lock_scene_frame::frame::scene::<VulkanRenderer>(
                state, vk, size, lp,
            );
            lock_els = l.Element;
        }

        // Drain the GLES renderer's deferred-destruction queue. In Vulkan mode
        // the GLES renderer only runs `prepare()` (bevy/iced/parallax + client
        // imports) and NEVER renders a frame, so the cleanup that GLES normally
        // performs inside `render()`/`Frame::finish` never runs. Dropped GLES
        // resources (textures, EGLImages, FBOs/RBOs — e.g. bevy surface textures
        // recreated on a zoom-resize) then accumulate in the destruction channel
        // and leak GPU memory (the compositor-PID VRAM growth seen only on the
        // Vulkan path; our own Vulkan device stays flat). Draining it each frame
        // is what the GLES compositor path gets for free via its own render.
        {
            use smithay::backend::renderer::Renderer;
            let mut r = gles_renderer.borrow_mut();
            if let Err(e) = r.as_mut().cleanup_texture_cache() {
                warn!("native vulkan: GLES cleanup_texture_cache failed: {e:?}");
            }
        }

        let scene_outputs: Vec<VkOutput> =
            scene_els.into_iter().map(VkOutput::Scene).collect();
        let lock_outputs: Vec<VkOutput> = lock_els.into_iter().map(VkOutput::Lock).collect();

        // Post-scene capture (native Vulkan copy). The capture must be the clean
        // SCENE, never lock content. During the Locked{pending} fade we mirror the
        // GLES tap path: render scene-only first (with capture targets set so
        // submit_frame copies the composed desktop into the registry entry
        // dmabufs), then render scene+lock for display. Outside the fade a single
        // pass suffices (and on a pure-scene Running frame we still set targets —
        // a no-op until a lock has created an entry).
        let capture_targets = || -> Vec<(
            smithay::backend::allocator::dmabuf::Dmabuf,
            Option<Rectangle<i32, Physical>>,
        )> {
            if !tap_post_scene {
                return Vec::new();
            }
            state
                .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                .as_ref()
                .map(|r| r.entry_dmabufs_for_output(output_id))
                .unwrap_or_default()
                .into_iter()
                .map(|(_, dmabuf, _, src)| (dmabuf, src))
                .collect()
        };

        if render_scene && render_lock {
            // Pass 1: scene-only, for capture (rendered, not queued).
            if !scene_outputs.is_empty() {
                let targets = capture_targets();
                if !targets.is_empty() {
                    let vk = ctx_ref.vulkan.as_mut().expect("vulkan_mode without renderer");
                    vk.set_capture_targets(targets);
                    if let Err(e) = ctx_ref.outputs[output_idx].drm_output.as_mut().unwrap().render_frame(
                        &mut *vk,
                        &scene_outputs,
                        [0.0, 0.0, 0.0, 1.0],
                        frame_flags,
                    ) {
                        error!("native vulkan capture render_frame failed: {e:?}");
                    }
                }
            }
            // Pass 2: scene + lock (front-to-back: lock on top), queued.
            let mut combined: Vec<VkOutput> =
                Vec::with_capacity(lock_outputs.len() + scene_outputs.len());
            combined.extend(lock_outputs);
            combined.extend(scene_outputs);
            if !combined.is_empty() {
                let vk = ctx_ref.vulkan.as_mut().expect("vulkan_mode without renderer");
                vk.set_capture_targets(Vec::new()); // never capture lock content
                match ctx_ref.outputs[output_idx].drm_output.as_mut().unwrap().render_frame(
                    &mut *vk,
                    &combined,
                    [0.0, 0.0, 0.0, 1.0],
                    frame_flags,
                ) {
                    Ok(result) => {
                        honor_needs_sync(&result);
                        last_result_empty = result.is_empty;
                    }
                    Err(e) => error!("native vulkan render_frame failed: {e:?}"),
                }
            }
        } else {
            // Single pass: Running (scene only) or fully-locked (lock only).
            let mut elements: Vec<VkOutput> =
                Vec::with_capacity(lock_outputs.len() + scene_outputs.len());
            elements.extend(lock_outputs);
            elements.extend(scene_outputs);
            if !elements.is_empty() {
                // Window/world-region capture renders the windows directly into
                // the entry after the scene (off-screen capable, chrome-free);
                // screen/full-screen capture keeps the blit via capture targets.
                let render_job = if render_scene && !render_lock {
                    compositor_y5_graphic_capture_interface::render::window_render_job(state)
                } else {
                    None
                };
                let targets = if render_scene && !render_lock && render_job.is_none() {
                    capture_targets()
                } else {
                    Vec::new()
                };
                {
                    let vk = ctx_ref.vulkan.as_mut().expect("vulkan_mode without renderer");
                    vk.set_capture_targets(targets);
                    match ctx_ref.outputs[output_idx].drm_output.as_mut().unwrap().render_frame(
                        &mut *vk,
                        &elements,
                        [0.0, 0.0, 0.0, 1.0],
                        frame_flags,
                    ) {
                        Ok(result) => {
                            honor_needs_sync(&result);
                            last_result_empty = result.is_empty;
                        }
                        Err(e) => error!("native vulkan render_frame failed: {e:?}"),
                    }
                }
                if let Some(job) = render_job {
                    let backdrop =
                        compositor_y5_graphic_capture_interface::render::capture_backdrop(
                            state, &job,
                        );
                    if let Some(mut dmabuf) = state
                        .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                        .as_ref()
                        .and_then(|r| r.entry_dmabuf(job.entry_id))
                    {
                        let vk = ctx_ref.vulkan.as_mut().expect("vulkan_mode without renderer");
                        compositor_y5_graphic_capture_interface::render::draw_windows_into_bg(
                            vk,
                            &mut dmabuf,
                            job.size,
                            &job.windows,
                            job.scale,
                            backdrop,
                        );
                    }
                }
            }
        }
    } else {
    match (render_scene, render_lock) {
        // ---------- Scene only (Running / Unlock) ----------
        (true, false) => {
            // ---- Build scene: scoped borrow_mut, dropped immediately. ----
            let scene = {
                let mut r = gles_renderer.borrow_mut();
                let prepared =
                    compositor_orchestration_draw_scene_frame::scene::prepare(state, r.as_mut(), size);
                let scene =
                    compositor_orchestration_draw_scene_frame::scene::scene(state, r.as_mut(), size, prepared);
                drop(r);
                scene
            };

            let wrapped: Vec<GlesElementWrapper<_>> =
                scene.Element.iter().map(GlesElementWrapper).collect();

            // ---- render_frame: hold RefMut for the lifetime of scene_result. ----
            let mut r = gles_renderer.borrow_mut();
            let scene_result = ctx_ref
                .outputs[output_idx]
                .drm_output
                .as_mut()
                .unwrap()
                .render_frame(&mut *r, &wrapped, [0.0, 0.0, 0.0, 1.0], frame_flags)
                .unwrap();
            honor_needs_sync(&scene_result);

            let scene_is_empty = scene_result.is_empty;

            // ---- Tap (post-scene): capture blit, inline with r held. ----
            // The safe pattern (carried from the original): extract everything
            // we need from scene_result BEFORE dropping r, perform the capture
            // INSIDE the same scope as r, then drop both together — a fresh
            // borrow_mut while scene_result is alive would alias.
            if tap_post_scene {
                if let Some(job) =
                    compositor_y5_graphic_capture_interface::render::window_render_job(state)
                {
                    // Window / world-region capture: render the captured windows
                    // directly into the entry (off-screen capable, chrome-free)
                    // with the GLES renderer that holds their buffers.
                    if let Some(mut dmabuf) = state
                        .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                        .as_ref()
                        .and_then(|reg| reg.entry_dmabuf(job.entry_id))
                    {
                        compositor_y5_graphic_capture_interface::render::draw_windows_into(
                            &mut *r,
                            &mut dmabuf,
                            job.size,
                            &job.windows,
                            job.scale,
                        );
                    }
                } else if let Some(registry) = &mut state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY) {
                    let entries = registry.entries_for_output(output_id);
                    let full_src = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), size);

                    for (entry_id, mut entry_tex, entry_size, src_override) in entries {
                        // Region captures blit their sub-rect of the composed
                        // scene; full captures blit the whole framebuffer.
                        let src = src_override.unwrap_or(full_src);
                        let result: Result<(), _> = (|| {
                            let mut entry_fb = r.bind(&mut entry_tex).map_err(
                                compositor_y5_graphic_capture_registry::registry::BlitErr::Bind,
                            )?;
                            scene_result
                                .blit_frame_result(
                                    entry_size,
                                    smithay::utils::Transform::Normal,
                                    Scale::from(1.0),
                                    &mut *r,
                                    &mut entry_fb,
                                    [src],
                                    std::iter::empty::<Id>(),
                                )
                                .map(|_sync| ())
                                .map_err(
                                    compositor_y5_graphic_capture_registry::registry::BlitErr::Blit,
                                )
                        })();
                        if let Err(e) = result {
                            warn!("capture blit failed: entry_id={entry_id:?} err={e:?}");
                        }
                    }
                }
            }

            drop(scene_result);
            drop(r);

            last_result_empty = scene_is_empty;
            visible_window = scene.visible_window;
        }

        // ---------- Scene + lock (Locked{pending:true} fade-in) ----------
        (true, true) => {
            // ---- Build scene: scoped. ----
            let scene = {
                let mut r = gles_renderer.borrow_mut();
                let prepared =
                    compositor_orchestration_draw_scene_frame::scene::prepare(state, r.as_mut(), size);
                let s =
                    compositor_orchestration_draw_scene_frame::scene::scene(state, r.as_mut(), size, prepared);
                drop(r);
                s
            };
            let scene_visible = scene.visible_window;
            let scene_wrapped: Vec<GlesElementWrapper<_>> =
                scene.Element.into_iter().map(GlesElementWrapper).collect();

            // ---- First render: scene only, used for the tap. ----
            let mut r = gles_renderer.borrow_mut();
            let scene_result = ctx_ref
                .outputs[output_idx]
                .drm_output
                .as_mut()
                .unwrap()
                .render_frame(&mut *r, &scene_wrapped, [0.0, 0.0, 0.0, 1.0], frame_flags)
                .unwrap();
            honor_needs_sync(&scene_result);

            // ---- Tap inline (same reasoning as above). The tap sits between
            //      Scene and Lock in the plan: it must never see lock content.
            if tap_post_scene {
                if let Some(job) =
                    compositor_y5_graphic_capture_interface::render::window_render_job(state)
                {
                    // Window / world-region capture: render the captured windows
                    // directly into the entry (off-screen capable, chrome-free)
                    // with the GLES renderer that holds their buffers.
                    if let Some(mut dmabuf) = state
                        .inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY)
                        .as_ref()
                        .and_then(|reg| reg.entry_dmabuf(job.entry_id))
                    {
                        compositor_y5_graphic_capture_interface::render::draw_windows_into(
                            &mut *r,
                            &mut dmabuf,
                            job.size,
                            &job.windows,
                            job.scale,
                        );
                    }
                } else if let Some(registry) = &mut state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE_REGISTRY) {
                    let entries = registry.entries_for_output(output_id);
                    let full_src = Rectangle::<i32, Physical>::from_loc_and_size((0, 0), size);

                    for (entry_id, mut entry_tex, entry_size, src_override) in entries {
                        // Region captures blit their sub-rect of the composed
                        // scene; full captures blit the whole framebuffer.
                        let src = src_override.unwrap_or(full_src);
                        let result: Result<(), _> = (|| {
                            let mut entry_fb = r.bind(&mut entry_tex).map_err(
                                compositor_y5_graphic_capture_registry::registry::BlitErr::Bind,
                            )?;
                            scene_result
                                .blit_frame_result(
                                    entry_size,
                                    smithay::utils::Transform::Normal,
                                    Scale::from(1.0),
                                    &mut *r,
                                    &mut entry_fb,
                                    [src],
                                    std::iter::empty::<Id>(),
                                )
                                .map(|_sync| ())
                                .map_err(
                                    compositor_y5_graphic_capture_registry::registry::BlitErr::Blit,
                                )
                        })();
                        if let Err(e) = result {
                            warn!("capture blit failed: entry_id={entry_id:?} err={e:?}");
                        }
                    }
                }
            }

            // ---- Done with scene_result; drop it AND r so we can re-borrow. ----
            drop(scene_result);
            drop(r);

            // ---- Build lock scene: fresh scoped borrow. ----
            let lock_scene = {
                let mut r = gles_renderer.borrow_mut();
                let lp = compositor_y5_lock_scene_frame::frame::prepare(state, r.as_mut(), size);
                let ls = compositor_y5_lock_scene_frame::frame::scene(state, r.as_mut(), size, lp);
                drop(r);
                ls
            };

            // ---- Build combined element list (element.combined: retired-by-
            //      plan once this path renders per-pass). ----
            let mut combined: Vec<OutputElement> =
                Vec::with_capacity(lock_scene.Element.len() + scene_wrapped.len());
            combined.extend(
                lock_scene
                    .Element
                    .into_iter()
                    .map(GlesElementWrapper)
                    .map(OutputElement::Lock),
            );
            combined.extend(scene_wrapped.into_iter().map(OutputElement::Scene));

            // ---- Second render: this is what gets queued. ----
            let mut r = gles_renderer.borrow_mut();
            let combined_result = ctx_ref
                .outputs[output_idx]
                .drm_output
                .as_mut()
                .unwrap()
                .render_frame(&mut *r, &combined, [0.0, 0.0, 0.0, 1.0], frame_flags)
                .unwrap();
            honor_needs_sync(&combined_result);

            last_result_empty = combined_result.is_empty;
            visible_window = scene_visible;

            drop(combined_result);
            drop(r);
        }

        // ---------- Lock only (fully Locked, no fade) ----------
        (false, true) => {
            let lock_scene = {
                let mut r = gles_renderer.borrow_mut();
                let lp = compositor_y5_lock_scene_frame::frame::prepare(state, r.as_mut(), size);
                let ls = compositor_y5_lock_scene_frame::frame::scene(state, r.as_mut(), size, lp);
                drop(r);
                ls
            };

            let wrapped: Vec<GlesElementWrapper<_>> =
                lock_scene.Element.iter().map(GlesElementWrapper).collect();

            let mut r = gles_renderer.borrow_mut();
            let lock_result = ctx_ref
                .outputs[output_idx]
                .drm_output
                .as_mut()
                .unwrap()
                .render_frame(&mut *r, &wrapped, [0.0, 0.0, 0.0, 1.0], frame_flags)
                .unwrap();
            honor_needs_sync(&lock_result);

            last_result_empty = lock_result.is_empty;

            drop(lock_result);
            drop(r);
        }

        // ---------- Empty plan (Sleep / Terminate) ----------
        (false, false) => {}
    }
    }

    // All RefMut guards on the renderer have been dropped by this point.
    // ---- present THIS output: queue its page-flip (or send empty-frame callbacks).
        if !last_result_empty {
            if present(ctx_ref, state, visible_window, output_idx) {
                any_queued = true;
            }
            state.schedule_redraw();
        } else {
            let output = ctx_ref.outputs[output_idx].output.clone();
            #[cfg(feature = "flip-estimate")]
            if ctx_ref.safety.estimate_pacing {
                // Estimate net active: hold the frame callbacks; `wire.frame`
                // delivers them at the estimated next vblank.
                compositor_kernel_graphic_draw_present_callbacks::callbacks::housekeeping(state);
                deferred = Some(FrameOutcome::EmptyDeferred {
                    output: output.clone(),
                    visible: visible_window.clone(),
                });
            } else {
                compositor_kernel_graphic_draw_present_callbacks::callbacks::send_window_frames(
                    state, &output, &visible_window,
                );
            }
            #[cfg(not(feature = "flip-estimate"))]
            compositor_kernel_graphic_draw_present_callbacks::callbacks::send_window_frames(
                state, &output, &visible_window,
            );
        }
    } // ---- end per-output render loop ----

    // Drawing done → clear the render-output seam and release the shared GPU state.
    state.inner.render_output = None;
    drop(gles_renderer);
    drop(binding);
    drop(ctx);

    // Housekeeping runs *every* execute() call, damage or no.
    compositor_kernel_graphic_draw_present_callbacks::callbacks::housekeeping(state);
    if any_queued {
        FrameOutcome::Queued
    } else {
        #[cfg(feature = "flip-estimate")]
        {
            deferred.unwrap_or(FrameOutcome::Idle)
        }
        #[cfg(not(feature = "flip-estimate"))]
        {
            FrameOutcome::Idle
        }
    }
}

/// Queue the rendered frame with presentation feedback and send frame
/// callbacks. (Ex scene.rs `refresh()`, recomposed from present.callbacks +
/// flip.queue.) Queue failure panics outside the session-resume window;
/// inside it the watchdog recovers and no frame callbacks are sent (the
/// original's abort shape). Returns whether a frame is in flight.
fn present(
    ctx_ref: &mut NativeRenderContext,
    state: &mut Loop,
    window_visible: Vec<smithay::desktop::Window>,
    output_idx: usize,
) -> bool {
    use compositor_kernel_scanout_flip_queue_base::queue::{queue, QueueOutcome};

    let current_output = ctx_ref.outputs[output_idx].output.clone();
    let feedback = compositor_kernel_graphic_draw_present_callbacks::callbacks::collect_feedback(
        &current_output,
        &window_visible,
    );

    let resuming = !(*state.inner.kernel.get(&compositor_orchestration_driver_resume_base::base::VBLANK_SEEN));
    // Scope the drm_output borrow so the `Failed` arm can tear the pipe down.
    let outcome = {
        let Some(drm_output) = ctx_ref.outputs[output_idx].drm_output.as_mut() else { return false };
        queue(drm_output, Some(feedback), resuming)
    };
    match outcome {
        QueueOutcome::Queued => {
            state.mark_render_queued();
            // Mark this pipe in-flight: the render loop skips it until its own
            // vblank scans this frame out, decoupling its cadence from the others.
            ctx_ref.outputs[output_idx].in_flight = true;
        }
        QueueOutcome::DeferredToWatchdog => {
            // No frame callbacks for this frame; the watchdog re-kicks.
            return false;
        }
        QueueOutcome::Failed => {
            // Fail-soft: this connector's flip failed → drop its scanout target so
            // the render loop skips it (it goes dark) while other outputs keep
            // running. Recovered on the next hotplug reconcile.
            ctx_ref.outputs[output_idx].drm_output = None;
            return false;
        }
    }

    compositor_kernel_graphic_draw_present_callbacks::callbacks::send_window_frames(
        state,
        &current_output,
        &window_visible,
    );
    compositor_kernel_graphic_draw_present_callbacks::callbacks::send_layer_frames(state, &current_output);
    true
}

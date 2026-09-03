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
use compositor_orchestration_draw_dispatch_frame::{ElementMeta, SceneDispatch};
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
    /// A scene element + its per-element metadata (space, …).
    Scene(VkScene, ElementMeta),
    Lock(VkLock),
}

impl Element for VkOutput {
    fn id(&self) -> &Id {
        match self { Self::Scene(e, _) => e.id(), Self::Lock(e) => e.id() }
    }
    fn current_commit(&self) -> CommitCounter {
        match self { Self::Scene(e, _) => e.current_commit(), Self::Lock(e) => e.current_commit() }
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        match self { Self::Scene(e, _) => e.src(), Self::Lock(e) => e.src() }
    }
    fn geometry(&self, s: Scale<f64>) -> Rectangle<i32, Physical> {
        match self { Self::Scene(e, _) => e.geometry(s), Self::Lock(e) => e.geometry(s) }
    }
    fn location(&self, s: Scale<f64>) -> Point<i32, Physical> {
        match self { Self::Scene(e, _) => e.location(s), Self::Lock(e) => e.location(s) }
    }
    fn transform(&self) -> Transform {
        match self { Self::Scene(e, _) => e.transform(), Self::Lock(e) => e.transform() }
    }
    fn damage_since(&self, s: Scale<f64>, c: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        match self { Self::Scene(e, _) => e.damage_since(s, c), Self::Lock(e) => e.damage_since(s, c) }
    }
    fn opaque_regions(&self, s: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        match self { Self::Scene(e, _) => e.opaque_regions(s), Self::Lock(e) => e.opaque_regions(s) }
    }
    fn alpha(&self) -> f32 {
        match self { Self::Scene(e, _) => e.alpha(), Self::Lock(e) => e.alpha() }
    }
    fn kind(&self) -> Kind {
        match self { Self::Scene(e, _) => e.kind(), Self::Lock(e) => e.kind() }
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
            Self::Scene(e, meta) => {
                // Tag this element's metadata so `render_texture_from_to`
                // restricts AA to world content.
                <VulkanRenderer as SceneDispatch>::set_element_meta(frame, *meta);
                e.draw(frame, src, dst, damage, opaque, cache)
            }
            Self::Lock(e) => {
                <VulkanRenderer as SceneDispatch>::set_element_meta(frame, ElementMeta::SCREEN);
                e.draw(frame, src, dst, damage, opaque, cache)
            }
        }
    }
    fn underlying_storage(&self, r: &mut VulkanRenderer) -> Option<UnderlyingStorage<'_>> {
        match self {
            Self::Scene(e, _) => e.underlying_storage(r),
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

/// Shortest rate-cap wait worth deferring for. A deferral costs a timerfd
/// wake-up plus a ping round-trip back through the event loop; under roughly a
/// millisecond that overhead exceeds the interval being enforced, so the cap
/// would cost more rate than it saves.
const CAP_DEFER_FLOOR: std::time::Duration = std::time::Duration::from_micros(1_000);

/// Consecutive `render_frame` failures after which a pipe is PARKED — skipped
/// entirely rather than retried every frame.
///
/// The failure this exists for is not transient. When smithay's bring-up
/// escalation drops the device to implicit modifiers, the Vulkan renderer cannot
/// create an image for the scanout buffer at all, so EVERY frame fails on EVERY
/// pipe. Before this, that produced a black screen plus an unbounded error stream
/// at frame rate, which buried the one line that actually explained it. 120 frames
/// is ~2s at 60Hz — long enough that a genuinely transient failure recovers by
/// itself and is never parked.
const RENDER_FAILURE_PARK: u32 = 120;

/// Restore the render/submit invariant after a frame that was rendered but will
/// NOT be queued.
///
/// `DrmCompositor` pushes a damage-history entry on every render that produced
/// damage (`renderer/damage/mod.rs`), but advances swapchain slot ages only in
/// `queue_frame`/`commit_frame` -> `swapchain.submitted`. The two must stay 1:1.
/// A render that is never queued leaves every OTHER slot's recorded age one lower
/// than its true age, so the tracker hands back LESS damage than that buffer
/// actually needs and stale regions survive — the "screen alternating between the
/// last two samples" artifact. Zeroing the ages discards the poisoned accounting:
/// the next render is a full redraw (age 0) and everything is consistent again.
///
/// ONLY for genuinely unsubmitted renders. An EMPTY render needs nothing: it
/// pushes no history and performs no submit, so it is already consistent. A
/// `queue_frame` whose DRM submit fails is also fine — `submitted()` ran first.
///
/// `FrameFlags::FORCE_PRESENT` (pre-emptive rendering) makes a would-be-empty
/// frame queueable, and needs nothing here either — it changes only
/// `PreparedFrame::is_empty`, not `plane_state.skip`, and `queue_frame` gates
/// `swapchain.submitted` on `skip`. So such a frame re-presents the current
/// framebuffer without pushing history OR advancing ages: still 1:1.
fn discard_unsubmitted_render(
    pipe: &compositor_kernel_native_context_render_base::render::OutputPipe,
) {
    if let Some(o) = pipe.drm_output.as_ref() {
        o.with_compositor(|c| c.reset_buffer_ages());
    }
}

/// Record one `render_frame` outcome and rate-limit the reporting.
///
/// Success clears the streak. Failure logs the FIRST one in full — that is the one
/// worth reading — then stays silent until the pipe is parked, which is announced
/// exactly once with what to look at. The counter is cleared by any success and by
/// a rebuild (`display.reconcile`), so a parked pipe comes back on the next
/// hotplug/resume reconcile.
fn note_render_result(
    pipe: &mut compositor_kernel_native_context_render_base::render::OutputPipe,
    err: Option<String>,
) {
    let Some(e) = err else {
        if pipe.render_failures > 0 {
            info!(
                "render recovered on connector={:?} crtc={:?} after {} consecutive failure(s)",
                pipe.connector, pipe.crtc, pipe.render_failures
            );
        }
        pipe.render_failures = 0;
        return;
    };
    pipe.render_failures = pipe.render_failures.saturating_add(1);
    match pipe.render_failures {
        1 => error!(
            "native vulkan render_frame failed on connector={:?} crtc={:?}: {e}",
            pipe.connector, pipe.crtc
        ),
        n if n == RENDER_FAILURE_PARK => error!(
            "connector={:?} crtc={:?} failed {n} consecutive frames — PARKING this pipe; \
             no further render attempts until a hotplug/resume reconcile rebuilds it. \
             Last error: {e}. A persistent failure here usually means the scanout buffer \
             sits on an IMPLICIT modifier, which the Vulkan renderer cannot import — check \
             for a preceding \"trying implicit modifiers\" escalation during bring-up.",
            pipe.connector, pipe.crtc
        ),
        _ => {}
    }
}

/// The world whose background is on screen this frame.
///
/// The same resolution the three prepare paths use when they state their facts:
/// the picker draws its OWN world's background, everything else draws the spawn
/// target's — including the lock screen, which falls back through it.
fn drawn_world(state: &compositor_orchestration_core_state_base::Loop) -> uuid::Uuid {
    match state.inner.worlds.active_id() == compositor_y5_picker_system_base::base::PICKER_WORLD {
        true => compositor_y5_picker_system_base::base::PICKER_WORLD,
        false => state.inner.worlds.spawn_target(),
    }
}

/// What that world's bundle asks of the engine. Neutral for a world with no
/// pipeline slot, which is the same answer as "no bundle" and the right one.
fn drawn_facts(
    state: &compositor_orchestration_core_state_base::Loop,
) -> compositor_pipeline_abi_worldset_base::base::Facts {
    let w = drawn_world(state);
    match state.inner.worlds.contains(w) {
        true => compositor_pipeline_world_system_base::base::facts(
            state.inner.worlds.get(w).storage(),
        ),
        false => Default::default(),
    }
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
    // Plane assignment is decided BEFORE this frame's scene exists, so it follows
    // the policy resolved LAST frame. Deliberately not the config's potential:
    // that would strip hardware planes (and the hardware cursor with them) the
    // moment any selector is armed, including on a desktop that is merely waiting
    // for a target and never tears.
    let mut frame_flags = compositor_kernel_scanout_plane_direct_base::direct::flags(
        compositor_support_smithay_state_tearing_gate::gate::tearing(),
    );
    // Pre-emptive rendering: never let a frame be reported empty, so the loop
    // flips every pass instead of parking — the tail of this function re-arms the
    // redraw latch only after a non-empty result. Default `Engaged` applies that
    // only while a section is governing; `Always` applies it unconditionally. See
    // `Config::preemptive`.
    //
    // FORCE_PRESENT and NOT `reset_buffer_ages` / `DRAW_ALL_ELEMENTS`: those two
    // force a full-screen redraw. Being pre-emptive must not mean doing more work
    // per frame — only doing it sooner — so damage stays honest and this changes
    // nothing but whether the prepared frame may be called empty.
    if compositor_model_environment_tearing_config::config::get()
        .preemptive
        .forces(compositor_support_smithay_state_tearing_gate::gate::governed())
    {
        frame_flags |= smithay::backend::drm::compositor::FrameFlags::FORCE_PRESENT;
    }
    // A bundle whose frame is cleared and recomposed whole must be handed the whole
    // element list, or the clear wipes what the damage tracker chose not to redraw.
    if drawn_facts(state).whole_frame {
        frame_flags |= smithay::backend::drm::compositor::FrameFlags::DRAW_ALL_ELEMENTS;
    }

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
        let key = compositor_orchestration_core_state_base::state::output_key(&ctx_ref.outputs[output_idx].output);
        if state.state.redraw.in_flight(&key) {
            continue;
        }
        // Already rendered for the current epoch: nothing has been requested of
        // this pipe since. A wake is a STALE signal — the ping only says "a
        // redraw was asked for since the last drain" — and the vblank path may
        // have rendered that request already (a commit that landed mid-flight is
        // rendered by the flip completion, and the ping it also armed then finds
        // an idle pipe). Without this, that wake re-rendered unchanged content,
        // and under a governing section `FORCE_PRESENT` flipped it: composites
        // above the paced client's commit rate, each a tear spent on nothing.
        // Same for the off-thread workers' pings. The epoch is the ground truth
        // the vblank path already trusts; every forced render bumps it
        // (`force_redraw`, `bump_redraw_epoch`), so rescues are unaffected.
        if !state.state.redraw.needs(&key) {
            continue;
        }
        // Rate cap / pacing gate: with async flips the completion event arrives
        // almost immediately, so nothing throttles the loop to the panel any
        // more. `min_interval` is the policy's ceiling (a multiple of refresh,
        // or the fixed target interval under `Paced`); holding the composite
        // back here is the whole point — an unpaced loop renders frames the beam
        // never reaches.
        //
        // This DEFERS, it does not drop. The caller already consumed the
        // `needs_redraw` latch and its lost-wakeup guard only covers `in_flight`
        // pipes, so a bare `continue` would leave nothing to wake the loop and
        // freeze the compositor until unrelated input scheduled a redraw. Arm a
        // one-shot timer for the remainder instead.
        {
            let pipe = &ctx_ref.outputs[output_idx];
            // The cap belongs to whichever section is in force, but the visible
            // set is not known until the scene is built — which happens after
            // this gate. So take the ceiling the PREVIOUS frame resolved from a
            // real scene, rather than resolving a fresh one from an approximate
            // `Scene` here (which got `TargetFocused` wrong). A one-frame lag on
            // a rate ceiling is immaterial; deferring the gate until after the
            // render is not, since the whole point is to skip the composite.
            let cap = pipe.cap_interval;
            // Measured START-to-START. Gating on the *end* of the previous frame
            // would enforce `min + composite`, not `min` — the cap and the render
            // would serialize instead of overlap, so even a cap far above the
            // achievable rate would slow the loop down.
            let held = match (cap, pipe.render_start) {
                (Some(min), Some(started)) => min.checked_sub(started.elapsed()),
                _ => None,
            }
            // Deferring costs a timerfd wake-up plus a ping round-trip through
            // the event loop. Below that cost the deferral is more expensive than
            // the interval it enforces — which is how a 20x cap (a 0.83ms
            // interval on 60Hz, i.e. nominally no cap at all) ended up throttling
            // harder than no cap. Round down to "render now" instead.
            .filter(|remaining| *remaining > CAP_DEFER_FLOOR);
            if let Some(remaining) = held {
                let now = std::time::Instant::now();
                // Re-arm only when no live timer covers this window; a deadline in
                // the past belongs to a timer that has already fired.
                if pipe.cap_wake.is_none_or(|deadline| deadline <= now) {
                    let token = loop_handle
                        .insert_source(
                            smithay::reexports::calloop::timer::Timer::from_duration(remaining),
                            move |_, _, state: &mut Loop| {
                                // force_redraw (not schedule_redraw): the latch may
                                // already read `true`, and only the unconditional
                                // ping restarts an otherwise idle cycle.
                                state.force_redraw();
                                smithay::reexports::calloop::timer::TimeoutAction::Drop
                            },
                        )
                        .ok();
                    if token.is_some() {
                        ctx_ref.outputs[output_idx].cap_wake = Some(now + remaining);
                    } else {
                        // Without a wake-up the loop would stall; rendering one
                        // frame early is strictly better than freezing.
                        warn!("rate-cap timer registration failed; compositing uncapped this frame");
                        ctx_ref.outputs[output_idx].cap_wake = None;
                    }
                }
                if ctx_ref.outputs[output_idx].cap_wake.is_some() {
                    continue;
                }
            }
        }
        // PARKED: this pipe has failed to render RENDER_FAILURE_PARK times running.
        // Retrying it every frame produced nothing but an unbounded error stream, so
        // stop drawing it until something rebuilds it (hotplug/resume reconcile
        // clears the counter). See `note_render_result`.
        if ctx_ref.outputs[output_idx].render_failures >= RENDER_FAILURE_PARK {
            continue;
        }
        // This pipe is being rendered for the CURRENT redraw epoch — stamp it so
        // neither its own vblank (`process_vblank`) nor a later `execute(All)` (the
        // epoch skip above) renders it again for the same request. Sampled once at
        // the top of the frame, so a redraw requested WHILE this renders leaves the
        // pipe behind and is serviced next time.
        state.state.redraw.rendering(&key);
        ctx_ref.outputs[output_idx].render_start = Some(std::time::Instant::now());
        // NO per-frame age reset here. Per-output damage state is already correct by
        // construction — one DrmCompositor, one damage tracker and one swapchain per
        // CRTC, nothing shared — and both renderers preserve the undamaged remainder
        // (Vulkan composites with LOAD_OP_LOAD + per-element scissors; GLES clears via
        // a scissored quad fill). Per-CRTC vblank pacing is exactly the case smithay is
        // designed for.
        //
        // The artifact this used to paper over was the render/submit invariant being
        // broken by the capture pre-render; see `discard_unsubmitted_render`. Resetting
        // ages every frame also REPLACES any in-flight slot with a fresh one, dropping
        // its GBM buffer — so it cost a full-resolution dmabuf reallocation and a
        // Vulkan re-import per output per frame, which is where the multi-monitor
        // frame-rate collapse came from.
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
    // The three states that put something on screen the live bundle did not draw.
    // Decided here because this is where the plan picks which of the three prepare
    // paths runs — the picker and the lock each have their own, so a hook inside
    // the scene's would not run for them at all. Overview is inside the scene pass,
    // hence the separate read. See `set::band_suppressed`.
    let suppressed = picker_active || render_lock || state.inner.overview().visible;
    let drawn = drawn_world(state);
    let facts = drawn_facts(state);
    // Cheap and unconditional: a field assignment on the renderer, and the value
    // every renderer-side gate reads.
    // WHICH OUTPUT this pass is for. The renderer's intermediate targets,
    // `content` and `history` are all sized from this pass's extent and filled by
    // this pass's composite; held as one set they were shared by every monitor.
    // The world's copy of what that pass produced is keyed the same way, so the
    // two cannot disagree about which monitor a set of rects describes.
    let out_key: std::sync::Arc<str> =
        std::sync::Arc::from(state.inner.render_output.clone().unwrap_or_default().as_str());
    if let Some(vk) = ctx_ref.vulkan.as_mut() {
        vk.set_render_output(&out_key);
        vk.reclaim_removed_outputs();
        vk.set_pipeline_facts(facts);
    }
    // EVERYTHING ELSE IS GATED. A desktop with no bundle — the overwhelmingly
    // common one, running the stock parallax — pays one token read here and
    // nothing more; the handovers below exist only for a pipeline.
    //
    // `holds`, not just `active`: the frame a bundle unloads has `active == false`
    // while the world is still carrying its set, grid and shares, so gating on
    // `active` alone would strand them. Deactivation therefore costs exactly one
    // more pass through here, which clears everything to `None`, and from the next
    // frame on this is a single comparison.
    let produced = ctx_ref.vulkan.as_ref().is_some_and(|vk| {
        let (c, w) = vk.shared_bands();
        vk.warp_grid().is_some() || c.is_some() || w.is_some()
    });
    if state.inner.worlds.contains(drawn)
        && (facts.active()
            || produced
            || compositor_pipeline_world_system_base::base::holds(
                state.inner.worlds.get(drawn).storage(),
            ))
    {
        // The three states that put something on screen the live bundle did not
        // draw. Decided here because this is where the plan picks which of the
        // three prepare paths runs — the picker and the lock each have their own,
        // so a hook inside the scene's would not run for them at all.
        compositor_pipeline_world_system_base::base::publish_suppressed(
            state.inner.worlds.get_mut(drawn).storage_mut(),
            suppressed,
        );
        //         // Drain what the renderer produced LAST frame into the world it belongs
        // to, for THIS output. One frame old, which is what the shared slot was.
        // One handover, for THIS output. The renderer produced all of it in this
        // pass, for this output, and splitting it into four keyed writes was four
        // chances to key one of them differently.
        let frame = compositor_pipeline_world_system_base::base::OutputFrame {
            world_set: ctx_ref.vulkan.as_mut().and_then(|vk| vk.take_world_set()),
            warp_grid: ctx_ref.vulkan.as_ref().and_then(|vk| vk.warp_grid()),
            content_share: ctx_ref.vulkan.as_ref().and_then(|vk| vk.shared_bands().0),
            windows_share: ctx_ref.vulkan.as_ref().and_then(|vk| vk.shared_bands().1),
        };
        compositor_pipeline_world_system_base::base::publish_frame(
            state.inner.worlds.get_mut(drawn).storage_mut(),
            &out_key,
            frame,
        );
    }


    // Connector property pass: colorimetry + link bit depth, once per pipe, after
    // smithay's first modeset has bound the connector (gated on a seen vblank so
    // the prop-only atomic commit references an ACTIVE connector). A TEST commit
    // validates first, so a rejected request can never blank the display.
    //
    // NOT gated on `hdr_active` any more. These are sticky properties inherited
    // from whoever owned the connector last — another VT's compositor, or an
    // earlier HDR session of our own. An SDR pipe must therefore actively reset
    // `Colorspace` to Default and clear the HDR metadata; leaving them alone is
    // what made SDR content render through BT.2020 (heavy red cast).
    if !ctx_ref.outputs[output_idx].props_applied && (*state.inner.kernel.get(&compositor_orchestration_driver_resume_base::base::VBLANK_SEEN)) {
        let depth = compositor_model_environment_config_base::base::get().depth;
        let want_bpc: u64 = if depth == 10 { 10 } else { 8 };
        let hdr_active = ctx_ref.outputs[output_idx].hdr_active;
        let conn = ctx_ref.outputs[output_idx].connector;
        let crtc = ctx_ref.outputs[output_idx].crtc;
        match crate::hdr::apply_connector_props(
            &ctx_ref.drm_fd,
            conn,
            &ctx_ref.outputs[output_idx].hdr_caps,
            hdr_active,
            want_bpc,
        ) {
            Ok(o) => {
                ctx_ref.outputs[output_idx].props_applied = true;
                info!("connector properties applied (colorimetry + max bpc)");
            }
            Err(e) => {
                ctx_ref.outputs[output_idx].props_applied = true; // don't retry every frame
                if hdr_active {
                    ctx_ref.outputs[output_idx].hdr_active = false;
                    let c = &ctx_ref.outputs[output_idx].hdr_caps;
                    compositor_model_stats_registry_base::base::set_hdr_info(
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
    }

    let mut last_result_empty = true;
    // THIS frame's per-element render states, kept only so `collect_feedback` can
    // report `ZeroCopy` per surface. Cloned out of the `RenderFrameResult` because
    // that borrows the renderer and is dropped well before `present` runs; the map
    // is one entry per element, so this is cheap next to the frame it describes.
    let mut frame_states: Option<smithay::backend::renderer::element::RenderElementStates> = None;
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
            let outputs: Vec<VkOutput> = scene
                .Element
                .into_iter()
                .zip(scene.meta)
                .map(|(e, aa)| VkOutput::Scene(e, aa))
                .collect();
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
                        frame_states = Some(result.states.clone());
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
            frame_states = Some(picker_result.states.clone());

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
        let mut scene_aa: Vec<ElementMeta> = Vec::new();
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
            scene_aa = s.meta;
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

        let scene_outputs: Vec<VkOutput> = scene_els
            .into_iter()
            .zip(scene_aa)
            .map(|(e, aa)| VkOutput::Scene(e, aa))
            .collect();
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
                    match ctx_ref.outputs[output_idx].drm_output.as_mut().unwrap().render_frame(
                        &mut *vk,
                        &scene_outputs,
                        [0.0, 0.0, 0.0, 1.0],
                        frame_flags,
                    ) {
                        Ok(_) => {}
                        Err(e) => error!("native vulkan capture render_frame failed: {e:?}"),
                    }
                    // This pass is for capture only — pass 2 below is what gets
                    // queued. Rendered-but-unqueued poisons the age accounting.
                    discard_unsubmitted_render(&ctx_ref.outputs[output_idx]);
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
                // `format!` the error into an owned String so no borrow of
                // `ctx_ref.outputs` escapes the match and the bookkeeping below can
                // take its own mutable borrow.
                let render_err = match ctx_ref.outputs[output_idx].drm_output.as_mut().unwrap().render_frame(
                    &mut *vk,
                    &combined,
                    [0.0, 0.0, 0.0, 1.0],
                    frame_flags,
                ) {
                    Ok(result) => {
                        honor_needs_sync(&result);
                        last_result_empty = result.is_empty;
                        frame_states = Some(result.states.clone());
                        None
                    }
                    Err(e) => Some(format!("{e:?}")),
                };
                note_render_result(&mut ctx_ref.outputs[output_idx], render_err);
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
                    let render_err = match ctx_ref.outputs[output_idx].drm_output.as_mut().unwrap().render_frame(
                        &mut *vk,
                        &elements,
                        [0.0, 0.0, 0.0, 1.0],
                        frame_flags,
                    ) {
                        Ok(result) => {
                            honor_needs_sync(&result);
                            last_result_empty = result.is_empty;
                            frame_states = Some(result.states.clone());
                            None
                        }
                        Err(e) => Some(format!("{e:?}")),
                    };
                    note_render_result(&mut ctx_ref.outputs[output_idx], render_err);
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
            // That first render fed the tap/capture only; the second render below is
            // the one that gets queued. Restore the render/submit invariant.
            discard_unsubmitted_render(&ctx_ref.outputs[output_idx]);

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
            frame_states = Some(combined_result.states.clone());
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
            frame_states = Some(lock_result.states.clone());

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
            if present(ctx_ref, state, visible_window, output_idx, frame_states.as_ref()) {
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
    states: Option<&smithay::backend::renderer::element::RenderElementStates>,
) -> bool {
    use compositor_kernel_scanout_flip_queue_base::queue::{queue, QueueOutcome};

    // Resolve which policy section governs this frame, from what is actually on
    // screen. Recomputed every frame off the drawn set, so panning away from a
    // target — even a frozen one — restores normal scheduling by itself; there
    // is no latched state to get stuck in.
    let active = {
        use compositor_support_smithay_state_tearing_gate::gate;
        use compositor_support_smithay_state_tearing_liveness::liveness;
        use compositor_support_smithay_state_tearing_pacer::pacer;
        use compositor_model_environment_tearing_select::select::{Exclusivity, Scene};
        use smithay::wayland::seat::WaylandFocus;

        // Both halves of the tag live on the SURFACE, and both are gathered over the
        // whole surface TREE: Mesa attaches `wp_tearing_control` to the surface it
        // presents to, which for many native games is a subsurface under the toplevel
        // while the heuristic stamped the toplevel. `Verdict::is_target` then resolves
        // the two ONCE, at the window — the client's statement wherever it spoke, the
        // heuristic only for the silence.
        //
        // One tag for both sections: pacing is another configurable layer over the same
        // "does this window own the cadence" question, not a separate claim. What keeps
        // an explicit setting above a client is `Selector::Always`, which ignores the
        // tag entirely.
        //
        // Xwayland is not offered the protocol (`wire.tearing::can_view`), so the hint
        // is always absent for an X11 window and none here is ever second-hand.
        let tagged = |w: &smithay::desktop::Window| {
            use smithay::wayland::compositor::{with_surface_tree_downward, TraversalAction};
            let mut verdict = pacer::Verdict::default();
            if let Some(s) = w.wl_surface() {
                with_surface_tree_downward(
                    s.as_ref(),
                    (),
                    |_, _, _| TraversalAction::DoChildren(()),
                    |_, states, _| {
                        if let Some(tag) = states.data_map.get::<pacer::TearingTag>() {
                            verdict.absorb(tag);
                        }
                    },
                    |_, _, _| true,
                );
            }
            verdict.is_target()
        };
        let focus = state.state.seat.seat.get_keyboard().and_then(|kb| kb.current_focus());
        // The overview overlay owns the whole content band, so the windows in
        // the drawn set are thumbnails inside compositor UI — not a client
        // presenting to the user. An empty scene disengages every section for as
        // long as it is open.
        //
        // Without this the cadence belongs to a window the user has just
        // navigated away from, and the failure is not theoretical: games
        // routinely stop drawing the moment they are covered, and a stalled
        // owner paces the overlay at the watchdog floor with every input-driven
        // redraw silenced. The thumbnails still take their frame callbacks
        // below, which is what keeps them live.
        //
        // The gate is one global across outputs, so this is deliberately not
        // per-output: the overlay is on the active monitor, but the answer to
        // "may a client own the cadence" has to be the same everywhere.
        //
        // `visible` alone, NOT `visible && overlay_ready()` the way the band
        // itself is gated — do not "correct" this to match. Between Super+Tab
        // and ready the normal canvas is still drawn, so the two conditions do
        // differ; but reaching ready is exactly what needs frames, since
        // `backdrop::arm` composites the freeze snapshot over those frames. Wait
        // for ready and a stalled target holds the gate through the one window
        // where releasing it is what lets the overlay open at all. The cost is a
        // few untorn frames during the transition.
        let scene = if state.inner.overview().visible {
            Scene::default()
        } else {
            Scene {
                target_visible: window_visible.iter().any(tagged),
                target_focused: focus.as_ref().is_some_and(|f| {
                    window_visible
                        .iter()
                        .filter(|w| tagged(w))
                        .any(|w| w.wl_surface().is_some_and(|s| s.as_ref() == f))
                }),
                any_focused: focus.is_some(),
                any_visible: !window_visible.is_empty(),
            }
        };

        let cfg = compositor_model_environment_tearing_config::config::get();
        let active = compositor_y5_graphic_tearing_resolve::resolve::active(&cfg, scene);

        // Stamp what the scene actually drew, for the gates that ask about
        // visibility. A stamp rather than a flag: the scene knows what it drew,
        // never what it didn't, so there is no moment at which every other
        // surface could be cleared.
        //
        // Skipped entirely for the gates that never read it — an ordinary
        // desktop (`None`) and `Focused` — since this is a `with_states` per
        // visible window per frame, and it would run for a stamp nothing looks
        // at. Resolved from THIS frame's exclusivity, which is known before the
        // gate is published, so the frame that first engages is already stamped.
        let frame = gate::advance_frame();
        let excl = active.exclusivity();
        if matches!(
            excl,
            Exclusivity::Exclusive | Exclusivity::ExclusiveFocused | Exclusivity::Visible
        ) {
            for w in &window_visible {
                if let Some(s) = w.wl_surface() {
                    smithay::wayland::compositor::with_states(s.as_ref(), |states| {
                        states.data_map.insert_if_missing(gate::VisibleSurface::default);
                        if let Some(v) = states.data_map.get::<gate::VisibleSurface>() {
                            v.stamp(frame);
                        }
                    });
                }
            }
        }

        // Translate the user-facing exclusivity into the gate the Wayland
        // dispatch enforces per commit. `Off` when the rule is not in force, so
        // the dispatch never has to know about policy, scenes or windows.
        let g = if excl.engaged(scene) {
            match excl {
                Exclusivity::None => gate::Gate::Off,
                Exclusivity::Exclusive => gate::Gate::Tagged,
                Exclusivity::ExclusiveFocused => gate::Gate::TaggedFocused,
                Exclusivity::Focused => gate::Gate::Focused,
                Exclusivity::Visible => gate::Gate::Visible,
            }
        } else {
            gate::Gate::Off
        };
        if gate::set(g) {
            info!("tearing: redraw gate = {g:?} ({excl:?}, scene={scene:?})");
            // The floor watchdog exists only to rescue a gated loop, so it lives
            // exactly as long as the gate does.
            if g == gate::Gate::Off {
                compositor_kernel_native_wire_watchdog_base::watchdog::disarm(
                    &state.loop_handle,
                    &mut ctx_ref.watchdog,
                );
            } else {
                compositor_kernel_native_wire_watchdog_base::watchdog::arm(
                    &state.loop_handle,
                    &mut ctx_ref.watchdog,
                );
            }
        }
        // Publish for the NEXT frame's plane assignment, and carry this frame's
        // rate ceiling forward for the next frame's cap gate.
        let refresh = compositor_kernel_scanout_timing_vblank_base::vblank::interval(
            &ctx_ref.outputs[output_idx].mode,
        );
        ctx_ref.outputs[output_idx].cap_interval = active.min_interval(refresh);
        // The watchdog floor is a rate like any other, so it is resolved against
        // this output's refresh here — the layer that owns the timer knows
        // nothing about modes.
        compositor_support_smithay_state_tearing_floor::floor::set(cfg.floor_interval(refresh));
        // Published for the next frame's pre-emptive decision, beside the plane
        // one below and for the same reason — both are read before a scene exists.
        gate::set_governed(!matches!(
            active,
            compositor_y5_graphic_tearing_resolve::resolve::Active::Default
        ));
        if gate::set_tearing(active.may_tear(refresh)) {
            info!(
                "tearing: planes {} for the next frame",
                if active.may_tear(refresh) { "OFF (composited)" } else { "ON (direct scanout)" }
            );
        }
        liveness::note_composite(&compositor_orchestration_core_state_base::state::output_key(
            &ctx_ref.outputs[output_idx].output,
        ));
        active
    };

    let current_output = ctx_ref.outputs[output_idx].output.clone();
    let feedback = compositor_kernel_graphic_draw_present_callbacks::callbacks::collect_feedback(
        &current_output,
        &window_visible,
        states,
    );

    // Per-frame tearing decision. The FrameFlags half (plane assignment on/off)
    // is a mode-level property owned by `plane.direct`; this is the flip half.
    // The two MUST agree: a promoted overlay or cursor plane combined with the
    // async flag is an illegal commit that the kernel rejects outright.
    let tear = {
        let now = std::time::Instant::now();
        let pipe = &mut ctx_ref.outputs[output_idx];
        let refresh = compositor_kernel_scanout_timing_vblank_base::vblank::interval(&pipe.mode);
        // Time left in the current refresh interval, extrapolated from the last
        // anchored retrace. `None` until this pipe has flipped once, which
        // `tear_now` reads as "unknown timing → prefer the clean frame".
        let until_vblank = pipe.last_vblank.map(|anchor| {
            compositor_kernel_scanout_timing_vblank_base::vblank::until_next(anchor, now, refresh)
        });
        // This frame is going out, so any armed cap wake-up is spent.
        pipe.cap_wake = None;
        // Gated on the SAME value that chose this frame's plane flags. They must
        // agree: an async flip on a frame that still has planes armed carries two
        // planes and the kernel rejects it. On the frame a target first appears
        // the flags are still from the previous resolution, so this yields one
        // ordinary vsync'd frame rather than a rejected commit.
        let tear = active.tear_now(until_vblank, refresh)
            && compositor_support_smithay_state_tearing_gate::gate::tearing();
        pipe.last_tear = tear;
        tear
    };

    let resuming = !(*state.inner.kernel.get(&compositor_orchestration_driver_resume_base::base::VBLANK_SEEN));
    // Scope the drm_output borrow so the `Failed` arm can tear the pipe down.
    let outcome = {
        let Some(drm_output) = ctx_ref.outputs[output_idx].drm_output.as_mut() else { return false };
        // Arm the flip mode for THIS commit. `queue_frame` submits synchronously
        // (y5's per-pipe `in_flight` guard keeps `pending_frame` empty), so the
        // set-then-queue ordering is race-free.
        drm_output.with_compositor(|c| c.set_tearing(tear));
        queue(drm_output, Some(feedback), resuming)
    };
    match outcome {
        QueueOutcome::Queued => {
            // In flight: the render loop skips this pipe until its own vblank scans
            // the frame out, decoupling its cadence from the others' — and the
            // schedule wakes the loop for a request only while some pipe is idle.
            state.state.redraw.queued(&compositor_orchestration_core_state_base::state::output_key(&ctx_ref.outputs[output_idx].output));
        }
        QueueOutcome::DeferredToWatchdog => {
            // Rendered but not submitted: same invariant break as the capture
            // pre-render, so discard the age accounting rather than let it skew.
            // (`Failed` below needs nothing — it drops the whole `drm_output`, and
            // the swapchain goes with it.)
            discard_unsubmitted_render(&ctx_ref.outputs[output_idx]);
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
    compositor_kernel_graphic_draw_present_cursor::cursor::send_frames(state, &current_output);
    true
}

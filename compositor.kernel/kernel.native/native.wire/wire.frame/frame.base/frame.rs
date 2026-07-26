//! Frame pacing wiring: the redraw ping source, the DRM vblank source, and
//! the idle kickstart. (Ex wire.rs `start()` — ping, vblank closure, idle.)
//!
//! The Law-7 timing nets wire in here, each under its DOUBLE gate (cargo
//! feature compiles the mechanism in; the live `ctx.safety` enable activates
//! it):
//! - `timing-throttle`: re-time vblanks buggy drivers deliver early;
//! - `flip-estimate`:   deliver frame callbacks for empty-damage frames at
//!                      the estimated next vblank instead of immediately;
//! - `timing-predict`:  refine that estimate with a presentation clock
//!                      (implies `flip-estimate`).

use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use compositor_kernel_native_render_execute_base::execute::FrameOutcome;
use smithay::backend::drm::DrmDeviceNotifier;
use smithay::reexports::calloop::ping::make_ping;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::LoopHandle;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use compositor_orchestration_core_state_base::state::StatusSession;
use compositor_orchestration_core_state_base::Loop;

#[cfg(feature = "flip-estimate")]
type EstimateSlot = Rc<RefCell<Option<smithay::reexports::calloop::RegistrationToken>>>;
#[cfg(feature = "timing-predict")]
type PredictClock =
    Rc<RefCell<compositor_kernel_scanout_timing_predict_base::predict::PresentationClock>>;

pub fn register(
    event_loop: &mut EventLoop<'static, Loop>,
    state: &mut Loop,
    drm_notifier: DrmDeviceNotifier,
    ctx_rc: Rc<RefCell<NativeRenderContext>>,
) {
    let refresh = compositor_kernel_scanout_timing_vblank_base::vblank::interval(
        &ctx_rc.borrow().pipe().mode,
    );

    #[cfg(feature = "flip-estimate")]
    let estimate_slot: EstimateSlot = Rc::new(RefCell::new(None));
    #[cfg(feature = "timing-predict")]
    let predict_clock: PredictClock = Rc::new(RefCell::new(
        compositor_kernel_scanout_timing_predict_base::predict::PresentationClock::new(refresh),
    ));

    // ---- Redraw ping: fired by schedule_redraw while the vblank cycle is idle.
    let (redraw_ping, redraw_ping_source) = make_ping().unwrap();
    state.state.redraw_ping = Some(redraw_ping);

    let context_ping = ctx_rc.clone();
    let loop_handle_ping = event_loop.handle();
    #[cfg(feature = "flip-estimate")]
    let estimate_ping = estimate_slot.clone();
    #[cfg(feature = "timing-predict")]
    let predict_ping = predict_clock.clone();
    event_loop
        .handle()
        .insert_source(redraw_ping_source, move |_, _, state| {
            // We were pinged because something called schedule_redraw while
            // the VBlank cycle was idle. Run the executor to restart the cycle.
            if state.take_needs_redraw() {
                // Sampled BEFORE execute, and that ordering is the whole point.
                // `execute` SETS `in_flight` on every pipe it queues, so sampling
                // afterwards cannot tell "a pipe was already flying and got
                // skipped" (the case this guards) from "I just queued a frame"
                // (the normal case) — it sees `true` either way and re-arms
                // unconditionally. The pending vblank then finds the latch set
                // and renders a second time, so every frame produces two.
                //
                // That was invisible while the parallax re-armed `needs_redraw`
                // each frame anyway; with exclusive pacing silencing every other
                // source it became the sole re-arm, and doubled the pacer's rate.
                let was_in_flight =
                    context_ping.borrow().outputs.iter().any(|p| p.in_flight);
                let outcome = compositor_kernel_native_render_execute_base::execute::execute(
                    context_ping.clone(),
                    loop_handle_ping.clone(),
                    state,
                    compositor_kernel_native_render_execute_base::execute::RenderScope::All,
                );
                // Lost-wakeup guard: if this ping fired while a flip was still in
                // flight, execute(All) SKIPPED that pipe (its `in_flight` guard) and
                // re-armed nothing, yet `take_needs_redraw()` above already consumed
                // the latch. That pipe's vblank runs `take_needs_redraw()` next and
                // would find it cleared → no re-render, no further flip, and the
                // parallax's non-pinging `schedule_redraw_post_vblank` cannot restart
                // an idle cycle: the loop freezes until the next input schedule_redraw.
                // Re-arm (no ping) so the pending vblank still re-renders.
                if was_in_flight {
                    // `rearm_redraw`, not the animation path: this must survive
                    // exclusive pacing or the lost-wakeup freeze comes back.
                    state.rearm_redraw();
                }
                handle_outcome(
                    outcome,
                    &loop_handle_ping,
                    refresh,
                    #[cfg(feature = "flip-estimate")]
                    &estimate_ping,
                    #[cfg(feature = "timing-predict")]
                    &predict_ping,
                    #[cfg(feature = "timing-predict")]
                    state.inner.start_time.elapsed(),
                );
            }
        })
        .unwrap();

    // ---- VBlank: decode -> (throttle gate) -> interpret -> feedback ->
    //      conditional render.
    let context_drm = ctx_rc.clone();
    let loop_handle_vblank = event_loop.handle();
    #[cfg(feature = "flip-estimate")]
    let estimate_vblank = estimate_slot.clone();
    #[cfg(feature = "timing-predict")]
    let predict_vblank = predict_clock.clone();
    #[cfg(feature = "timing-throttle")]
    let throttle = Rc::new(RefCell::new(
        compositor_kernel_scanout_timing_throttle_base::throttle::VblankThrottle::new(),
    ));
    event_loop
        .handle()
        .insert_source(drm_notifier, move |event, event_meta, state| {
            use compositor_kernel_drm_loop_notifier_base::notifier::{decode, DecodedDrmEvent};

            match decode(event, event_meta) {
                DecodedDrmEvent::Error(error) => {
                    // The hosted compositor surfaces device errors here; the
                    // session lifecycle owns pause/resume, so an error outside
                    // it is not self-recovering.
                    abort!("DRM device error: {error}");
                }
                DecodedDrmEvent::VBlank {
                    pipe: crtc,
                    time,
                    sequence,
                } => {
                    if let StatusSession::Paused = state.inner.status_session {
                        return;
                    }

                    // Law-7 throttle gate: buggy-driver early vblanks are
                    // re-timed; the deferred delivery re-enters process_vblank.
                    #[cfg(feature = "timing-throttle")]
                    if context_drm.borrow().safety.vblank_throttle {
                        let stamp_now = state.inner.start_time.elapsed();
                        let ctx_for_deliver = context_drm.clone();
                        let handle_for_deliver = loop_handle_vblank.clone();
                        #[cfg(feature = "flip-estimate")]
                        let est_for_deliver = estimate_vblank.clone();
                        #[cfg(feature = "timing-predict")]
                        let pred_for_deliver = predict_vblank.clone();
                        let deferred = throttle.borrow_mut().throttle(
                            &loop_handle_vblank,
                            refresh,
                            time.unwrap_or(stamp_now),
                            move |state: &mut Loop| {
                                process_vblank(
                                    &ctx_for_deliver,
                                    &handle_for_deliver,
                                    state,
                                    time,
                                    sequence,
                                    crtc,
                                    refresh,
                                    #[cfg(feature = "flip-estimate")]
                                    &est_for_deliver,
                                    #[cfg(feature = "timing-predict")]
                                    &pred_for_deliver,
                                );
                            },
                        );
                        if deferred {
                            return;
                        }
                    }

                    process_vblank(
                        &context_drm,
                        &loop_handle_vblank,
                        state,
                        time,
                        sequence,
                        crtc,
                        refresh,
                        #[cfg(feature = "flip-estimate")]
                        &estimate_vblank,
                        #[cfg(feature = "timing-predict")]
                        &predict_vblank,
                    );
                }
            }
        })
        .unwrap();

    // ---- Kickstart the very first frame to initiate the cycle.
    let context_init = ctx_rc;
    // The exclusive-pacing floor watchdog is NOT registered here: it is armed on
    // the transition into gate engagement and dropped on the way out, by
    // `wire.watchdog`. See that crate for why.

    let loop_handle_init = event_loop.handle();
    #[cfg(feature = "flip-estimate")]
    let estimate_init = estimate_slot;
    #[cfg(feature = "timing-predict")]
    let predict_init = predict_clock;
    event_loop.handle().insert_idle(move |state| {
        let outcome = compositor_kernel_native_render_execute_base::execute::execute(
            context_init,
            loop_handle_init.clone(),
            state,
            compositor_kernel_native_render_execute_base::execute::RenderScope::All,
        );
        handle_outcome(
            outcome,
            &loop_handle_init,
            refresh,
            #[cfg(feature = "flip-estimate")]
            &estimate_init,
            #[cfg(feature = "timing-predict")]
            &predict_init,
            #[cfg(feature = "timing-predict")]
            state.inner.start_time.elapsed(),
        );
    });
}

/// One vblank: feedback for the completed frame, predict-clock update,
/// pending-estimate disarm (a real vblank supersedes the estimate), and the
/// conditional re-render.
#[allow(clippy::too_many_arguments)]
fn process_vblank(
    ctx_rc: &Rc<RefCell<NativeRenderContext>>,
    loop_handle: &LoopHandle<'static, Loop>,
    state: &mut Loop,
    time: Option<Duration>,
    sequence: u64,
    crtc: smithay::reexports::drm::control::crtc::Handle,
    refresh: Duration,
    #[cfg(feature = "flip-estimate")] estimate_slot: &EstimateSlot,
    #[cfg(feature = "timing-predict")] predict_clock: &PredictClock,
) {
    *state.inner.kernel.get_mut(&compositor_orchestration_driver_resume_base::base::VBLANK_SEEN_MUT) = true;

    let mut ctx = ctx_rc.borrow_mut();

    // Route the VBlank to the pipe whose CRTC flipped. If NO pipe matches, this is a
    // LATE flip completion from a CRTC whose pipe was just pruned (a monitor
    // deactivate / hotplug removed the pipe and freed its CRTC while a flip was still
    // queued on it). There is nothing to account it against — DROP it. Never fall
    // back to `outputs[0]`: clearing the primary's `in_flight` and popping its
    // feedback for someone else's flip corrupts the primary's flip state, causing a
    // double-queue that fails the primary's scanout and tears it down (both-black).
    let Some(idx) = ctx.outputs.iter().position(|p| p.crtc == crtc) else {
        return;
    };

    // This pipe's flip completed → it is no longer in flight. The re-render below
    // (on `needs_redraw`) will now redraw THIS output; other pipes still in flight
    // stay skipped until their own vblank, so each output paces to its own refresh.
    ctx.outputs[idx].in_flight = false;
    // Phase reference for the tearing policy's "time until the next vblank".
    //
    // Anchored to the retrace the kernel timestamped, NOT to when we observed
    // the event: an async (tearing) flip completes mid-scanout, so its event
    // arrival is not a vblank at all, and anchoring on it would corrupt the
    // phase. `anchor` recovers the true instant by measuring our dispatch delay
    // against CLOCK_MONOTONIC — the clock DRM stamps events with, and the one
    // `Instant` reads, which is why the two can be related at all. Note this
    // must NOT use `start_time.elapsed()`: that is a since-launch clock, a
    // different epoch entirely.
    ctx.outputs[idx].last_vblank = Some(compositor_kernel_scanout_timing_vblank_base::vblank::anchor(
        std::time::Instant::now(),
        time,
        compositor_kernel_scanout_timing_vblank_base::vblank::monotonic_now(),
    ));

    // 1. Pop presentation feedback for the frame that just hit screen. No output
    //    during a monitor-switch teardown window → nothing to pop.
    let pending_feedback = match ctx.outputs[idx].drm_output.as_mut() {
        Some(o) => compositor_kernel_scanout_flip_feedback_base::feedback::pop(o),
        None => None,
    };

    // Per-output present rate for the FPS overlay: count only real page-flip
    // completions on THIS pipe (a dropped frame — a vblank with no new buffer —
    // doesn't increment), keyed by output. The overlay samples the delta.
    if matches!(pending_feedback, Some(Some(_))) {
        let key =
            compositor_orchestration_core_state_base::state::output_key(&ctx.outputs[idx].output);
        compositor_model_stats_registry_base::base::present(&key);
    }

    // A torn frame has no predictable next refresh — it was applied mid-scanout
    // rather than at a retrace — so report `Unknown` rather than the panel's
    // fixed interval, which would be a wrong prediction rather than a missing one.
    let tore = ctx.outputs[idx].last_tear;
    let refresh_rate = if tore {
        smithay::wayland::presentation::Refresh::Unknown
    } else {
        compositor_kernel_scanout_timing_vblank_base::vblank::refresh_interval(&ctx.outputs[idx].mode)
    };
    // Per-output refresh interval — the pacing (empty-frame estimate delay) must
    // use the interval of the output that ACTUALLY flipped, not the global primary
    // `refresh`. Otherwise a high-refresh output is paced at a slower neighbour's
    // rate. `refresh` (the primary's, from register()) is retained only for the
    // throttle gate above, which is feature-gated off in the shipping build.
    let this_refresh =
        compositor_kernel_scanout_timing_vblank_base::vblank::interval(&ctx.outputs[idx].mode);
    drop(ctx);

    let stamp = compositor_kernel_scanout_timing_vblank_base::vblank::interpret(
        time,
        sequence,
        state.inner.start_time.elapsed(),
    );

    #[cfg(feature = "timing-predict")]
    predict_clock.borrow_mut().presented(stamp.time);

    // A real vblank supersedes any pending estimated delivery.
    #[cfg(feature = "flip-estimate")]
    if let Some(token) = estimate_slot.borrow_mut().take() {
        compositor_kernel_scanout_flip_estimate_base::estimate::disarm(loop_handle, token);
    }

    state.mark_vblank_arrived();

    // 2. Fire presentation callbacks for that completed frame.
    if let Some(Some(mut feedback)) = pending_feedback {
        compositor_kernel_scanout_flip_feedback_base::feedback::presented(
            &mut feedback,
            stamp.time,
            refresh_rate,
            stamp.sequence,
            compositor_kernel_graphic_draw_present_callbacks::callbacks::hw_flip_kind(tore),
        );
    }

    // 3. If anything has requested a redraw since last time, render now — but
    //    ONLY this output (the one that flipped). Other outputs are re-rendered
    //    on their OWN vblanks, so a fast monitor is never paced by a slow one.
    let was_needed = state.take_needs_redraw();
    if was_needed {
        let outcome = compositor_kernel_native_render_execute_base::execute::execute(
            ctx_rc.clone(),
            loop_handle.clone(),
            state,
            compositor_kernel_native_render_execute_base::execute::RenderScope::Crtc(crtc),
        );
        handle_outcome(
            outcome,
            loop_handle,
            this_refresh,
            #[cfg(feature = "flip-estimate")]
            estimate_slot,
            #[cfg(feature = "timing-predict")]
            predict_clock,
            #[cfg(feature = "timing-predict")]
            stamp.time,
        );
    }
}

/// Act on the executor's outcome. Without the `flip-estimate` net this is a
/// no-op (Queued/Idle carry no pacing obligation).
#[allow(unused_variables)]
fn handle_outcome(
    outcome: FrameOutcome,
    loop_handle: &LoopHandle<'static, Loop>,
    refresh: Duration,
    #[cfg(feature = "flip-estimate")] estimate_slot: &EstimateSlot,
    #[cfg(feature = "timing-predict")] predict_clock: &PredictClock,
    #[cfg(feature = "timing-predict")] now: Duration,
) {
    match outcome {
        FrameOutcome::Queued | FrameOutcome::Idle => {}
        #[cfg(feature = "flip-estimate")]
        FrameOutcome::EmptyDeferred { output, visible } => {
            // Delay: predicted next presentation when the predict net is in,
            // one refresh interval otherwise.
            #[cfg(feature = "timing-predict")]
            let delay = predict_clock
                .borrow()
                .next_presentation(now)
                .saturating_sub(now);
            #[cfg(not(feature = "timing-predict"))]
            let delay = refresh;

            let mut slot = estimate_slot.borrow_mut();
            if let Some(token) = slot.take() {
                compositor_kernel_scanout_flip_estimate_base::estimate::disarm(loop_handle, token);
            }
            let token = compositor_kernel_scanout_flip_estimate_base::estimate::arm(
                loop_handle,
                delay,
                move |state: &mut Loop| {
                    compositor_kernel_graphic_draw_present_callbacks::callbacks::send_window_frames(
                        state, &output, &visible,
                    );
                    compositor_kernel_graphic_draw_present_callbacks::callbacks::send_layer_frames(state, &output);
                },
            );
            *slot = Some(token);
        }
    }
}

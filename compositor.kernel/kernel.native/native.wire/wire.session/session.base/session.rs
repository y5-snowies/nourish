//! Session pause/resume wiring: registers the seat notifier source and maps
//! its events through the seat lifecycle protocols onto the hosted pipe.
//! (Ex wire.rs `start()` session closure — the only crate besides the other
//! wire.* siblings that names `Loop`, Law 4.)
//!
//! Completion-pass addition: held modifiers are cleared on PAUSE (the same
//! VT-switch stuck-modifier problem the winit path fixed; the keys-up events
//! are consumed by the other VT, so the compositor must forget them).

use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use smithay::backend::session::libseat::LibSeatSessionNotifier;
use smithay::reexports::calloop::EventLoop;
use smithay::utils::{Logical, Point};
use std::cell::RefCell;
use std::rc::Rc;
use compositor_orchestration_core_state_base::state::StatusSession;
use compositor_orchestration_core_state_base::Loop;

pub fn register(
    event_loop: &mut EventLoop<'static, Loop>,
    session_notifier: LibSeatSessionNotifier,
    ctx_rc: Rc<RefCell<NativeRenderContext>>,
    render_kick: impl Fn(Rc<RefCell<NativeRenderContext>>, &mut Loop) + Clone + 'static,
) {
    let session_context = ctx_rc;
    let session_loop_handle = event_loop.handle();

    event_loop
        .handle()
        .insert_source(session_notifier, move |event, _, state| {
            let mut ctx = session_context.borrow_mut();
            let ctx_ref = &mut *ctx;

            match event {
                smithay::backend::session::Event::PauseSession => {
                    state.inner.status_session = StatusSession::Paused;
                    info!("Session paused (TTY switch away)");

                    // Seat de-activating — stop and discard any active capture.
                    compositor_y5_graphic_capture_interface::interface::stop_and_discard(state);

                    // The keys-up for anything held at switch time will be
                    // consumed by the other VT — forget held modifiers now.
                    compositor_kernel_graphic_seat_modifier_clear::clear::clear_held_modifiers(state);

                    // Pause protocol: display first, then input (seat.lifecycle).
                    let manager = ctx_ref.drm_output_manager.clone();
                    let libinput = &mut ctx_ref.libinput_context;
                    compositor_kernel_seat_lifecycle_pause_base::pause::pause(
                        || {
                            compositor_kernel_scanout_surface_output_base::output::pause(
                                &mut manager.borrow_mut(),
                            )
                        },
                        || libinput.suspend(),
                    );

                    *state.inner.kernel.get_mut(&compositor_orchestration_driver_resume_base::base::VBLANK_SEEN_MUT) = false;

                    if let Some(token) = state.inner.kernel.get_mut(&compositor_orchestration_driver_resume_base::base::RESUME_WATCHDOG_MUT).take() {
                        state.loop_handle.remove(token);
                    }
                }
                smithay::backend::session::Event::ActivateSession => {
                    info!("Session activated");
                    state.inner.status_session = StatusSession::Active;

                    // Resume protocol (seat.lifecycle): input, activate
                    // (forced reclaiming modeset), surface reset, buffer
                    // reset, remap. Step failures are the self-recovering
                    // class — the watchdog drives recovery.
                    {
                        let manager = ctx_ref.drm_output_manager.clone();
                        let space = &mut state.inner.space_state_mut().state;
                        // Remap EVERY live output back at its current global-space
                        // position (multi-output: not just the primary — a secondary
                        // monitor would otherwise stay unmapped/dark after a VT switch).
                        // Its existing geometry loc is the layout to restore.
                        let remap_list: Vec<(smithay::output::Output, Point<i32, Logical>)> = ctx_ref
                            .outputs
                            .iter()
                            .map(|p| {
                                let loc = space
                                    .output_geometry(&p.output)
                                    .map(|g| g.loc)
                                    .unwrap_or_else(|| Point::from((0, 0)));
                                (p.output.clone(), loc)
                            })
                            .collect();
                        let libinput = &mut ctx_ref.libinput_context;
                        // `Option` (per pipe) because a monitor switch briefly tears an
                        // output down before rebuilding. That can't overlap this session
                        // callback (calloop runs sources serially), but the resume path
                        // is the self-recovering class — skip a missing surface rather
                        // than panic. Direct `outputs` field access (not `pipe_mut()`)
                        // so this borrows only `outputs`, leaving the other `ctx_ref`
                        // fields the resume closures capture (libinput, …) borrowable.
                        let pipes = &mut ctx_ref.outputs;

                        compositor_kernel_seat_lifecycle_resume_base::resume::resume(
                            compositor_kernel_seat_lifecycle_resume_base::resume::ResumeSteps {
                                resume_input: || {
                                    libinput
                                        .resume()
                                        .map_err(|e| format!("libinput resume failed: {e:?}"))
                                },
                                activate_display: |force| {
                                    compositor_kernel_scanout_surface_output_base::output::activate(
                                        &mut manager.borrow_mut(),
                                        force,
                                    )
                                },
                                // Reset EVERY live pipe's surface, not just the primary,
                                // and clear its in-flight flag: any frame queued before
                                // the switch will never deliver a vblank, so the flag
                                // must not wedge the pipe out of the render loop.
                                reset_surface: || {
                                    let mut result = Ok(());
                                    for p in pipes.iter_mut() {
                                        p.in_flight = false;
                                        if let Some(o) = p.drm_output.as_mut() {
                                            if let Err(e) = compositor_kernel_scanout_surface_output_base::output::reset(o) {
                                                result = Err(e);
                                            }
                                        }
                                    }
                                    result
                                },
                                reset_buffers: || {},
                                remap_output: || {
                                    for (output, loc) in &remap_list {
                                        space.map_output(output, *loc);
                                    }
                                    space.refresh();
                                },
                            },
                        );
                    }
                    drop(ctx);

                    // Arm the watchdog: kick a full render every frame until a
                    // REAL vblank (flag set by wire.frame) arrives, then drop
                    // itself. Registration failure panics inside arm().
                    *state.inner.kernel.get_mut(&compositor_orchestration_driver_resume_base::base::VBLANK_SEEN_MUT) = false;
                    if let Some(token) = state.inner.kernel.get_mut(&compositor_orchestration_driver_resume_base::base::RESUME_WATCHDOG_MUT).take() {
                        state.loop_handle.remove(token);
                    }

                    let ctx = session_context.clone();
                    let kick = render_kick.clone();
                    let token = compositor_kernel_seat_lifecycle_resume_base::resume::watchdog::arm(
                        &session_loop_handle,
                        |state: &mut Loop| (*state.inner.kernel.get(&compositor_orchestration_driver_resume_base::base::VBLANK_SEEN)),
                        |state: &mut Loop| {
                            *state.inner.kernel.get_mut(&compositor_orchestration_driver_resume_base::base::RESUME_WATCHDOG_MUT) = None;
                        },
                        move |state: &mut Loop| kick(ctx.clone(), state),
                    );
                    *state.inner.kernel.get_mut(&compositor_orchestration_driver_resume_base::base::RESUME_WATCHDOG_MUT) = Some(token);
                }
            }
        })
        .unwrap();
}

//! Device plug/unplug wiring: registers the retained udev watcher and routes
//! typed events to the host's reaction (`native.plugin/plugin.route`).
//! NEW capability of the restructure: previously the UdevBackend was dropped
//! after the initial lookup and hotplug events never flowed.

use compositor_kernel_gpu_registry_node_base::node::NodeRegistry;
use compositor_kernel_native_context_render_base::render::NativeRenderContext;
use compositor_kernel_native_context_topology_base::topology::Topology;
use compositor_kernel_udev_loop_watch_base::watch::UdevWatch;
use smithay::reexports::calloop::EventLoop;
use std::cell::RefCell;
use std::rc::Rc;
use compositor_orchestration_core_state_base::Loop;

pub fn register(
    event_loop: &mut EventLoop<'static, Loop>,
    watch: UdevWatch,
    registry: Rc<RefCell<NodeRegistry>>,
    topology: Rc<RefCell<Topology>>,
    ctx_rc: Rc<RefCell<NativeRenderContext>>,
) {
    event_loop
        .handle()
        .insert_source(watch, move |event, _, state| {
            let decoded = compositor_kernel_udev_loop_event_base::event::decode(event);
            info!("udev event received: {decoded:?}");
            let rank = compositor_kernel_graphic_preference_gpu_rank::rank::get();
            let reconcile = compositor_kernel_native_plugin_route_base::route::route(
                decoded,
                &mut registry.borrow_mut(),
                &mut topology.borrow_mut(),
                &rank,
                &ctx_rc,
            );
            // A topology change on the driven device → drive the best connected
            // output per preferences (fail over / recover), or go dark + wait. Runs
            // in this udev dispatch (not the vblank callback), so the modeset is safe.
            if reconcile {
                if let Some(change) = compositor_kernel_native_context_display_reconcile::reconcile::reconcile(state, &ctx_rc) {
                    use compositor_orchestration_event_output_base::output::OutputChange;
                    // Fire the output-presence lifecycle event (event-driven — once
                    // per real transition) on the ACTIVE world's router: that is the
                    // world whose systems are dispatched (per-frame, and on the dark
                    // tick). Background worlds re-read the snapshot token on enable.
                    let active = state.inner.worlds.active_id();
                    compositor_orchestration_event_output_base::output::broadcast(
                        state.inner.worlds.get_mut(active).channels(),
                        change,
                    );
                    // Drive the dark control-plane tick across the dark window: arm a
                    // re-arming loop timer on WentDark (so RPC + the just-queued event
                    // dispatch progress with no rendering), drop it on Recovered.
                    // Capturing requires an output — stop any in-progress capture
                    // the moment the display goes away. (Capture is a Loop-level hook,
                    // not a World system, so the emitter stops it directly.)
                    if change == OutputChange::WentDark {
                        compositor_y5_graphic_capture_interface::interface::stop_and_discard(state);
                    }
                    match change {
                        OutputChange::WentDark if ctx_rc.borrow().dark_tick.is_none() => {
                            let token = state
                                .loop_handle
                                .insert_source(
                                    smithay::reexports::calloop::timer::Timer::immediate(),
                                    |_, _, state: &mut Loop| {
                                        compositor_orchestration_pump_dark_base::dark::pump(state);
                                        smithay::reexports::calloop::timer::TimeoutAction::ToDuration(
                                            std::time::Duration::from_millis(100),
                                        )
                                    },
                                )
                                .expect("dark control-plane tick registration failed");
                            ctx_rc.borrow_mut().dark_tick = Some(token);
                        }
                        OutputChange::Recovered => {
                            if let Some(token) = ctx_rc.borrow_mut().dark_tick.take() {
                                state.loop_handle.remove(token);
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Refresh the rim's display snapshot after any topology change so the
            // lid policy sees current external/internal presence.
            let active = ctx_rc.borrow().pipe().connector;
            let ctx = ctx_rc.borrow();
            let manager = ctx.drm_output_manager.borrow();
            let snap = compositor_kernel_native_context_display_base::base::compute(
                manager.device(),
                active,
            );
            drop(manager);
            drop(ctx);
            *state
                .inner
                .kernel
                .get_mut(&compositor_orchestration_driver_lid_base::base::DISPLAY_SNAPSHOT_MUT) =
                snap;
        })
        .unwrap();
}

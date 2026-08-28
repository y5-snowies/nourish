use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_support_system_world_frame_base::base::FramePlan;
use compositor_y5_notify_present_base::base::NotifyFrame;

/// What one output's systems tick produced: the active world's remaining draw
/// plan (the pass bridges what it knows), and the kernel host's notification
/// pill for this output, already taken out of it.
pub struct Ticked {
    pub plan: FramePlan,
    pub notify: Option<NotifyFrame>,
}

/// The main scene's per-frame rim hooks, then the active world's systems tick.
pub fn hooks(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) -> Ticked {
    compositor_y5_window_lifecycle_interface::interface::hook(state, renderer);
    // Promote any disk-restored placeholders (spawn-target world) into visible
    // placeholders — needs the renderer, so it can't happen at rehydrate time.
    compositor_y5_placeholder_interface_base::interface::promote_restored(state, renderer);
    // Session identities: seed the store from persisted placeholders once, then
    // carry client renames onto every placeholder holding the old key.
    compositor_y5_placeholder_interface_base::interface::reconcile_sessions(state);
    compositor_y5_surface_draw_hook::wgpu::hook(state, renderer, size);
    compositor_y5_graphic_capture_interface::interface::per_frame(state, renderer, size);
    // Reconcile the align/distribute selection toolbar against the live selection.
    compositor_y5_select_overlay_interface::interface::per_frame(state, renderer, size);
    // Debug FPS overlay (top-right): measures the composited-frame rate.
    compositor_y5_surface_draw_fps::fps::per_frame(state, renderer, size);
    // Reconcile the sticky touch pane against `inner.touch.pane_world`.
    compositor_y5_touch_pane_create::create::per_frame(state, renderer, size);
    // Reconcile the on-screen keyboard against `OSK.open`.
    compositor_y5_osk_board_create::create::per_frame(state, renderer, size);
    // Reconcile the empty-canvas guide popups (context menu + help panel) against
    // `GuideState` — the input rim only writes the desire, this builds it.
    compositor_y5_guide_menu_create::create::per_frame(state, renderer);
    compositor_y5_guide_menu_hover::hover::per_frame(state, size);
    compositor_y5_guide_help_create::create::per_frame(state, renderer, size);
    // …and the inline shader editor the menu's third entry opens. Also the push of
    // what it displays, which is why it runs every frame rather than only on the
    // open/close edge.
    compositor_y5_guide_shader_create::create::per_frame(state, renderer, size);
    compositor_orchestration_bus_legacy_base::legacy::drain(state, |l| &mut l.inner.bus);
    let ticked = world_tick(state, renderer, size);

    // A two-finger glide pans the camera without routing through the physical
    // cursor accumulator, so the first post-glide pointer motion snaps the cursor
    // across the whole pan. Re-seat the accumulator on the cursor's on-screen
    // position each frame (world location untouched) so that motion continues
    // seamlessly. Runs after `update()` so it sees this frame's pan/coast step.
    compositor_orchestration_seat_pointer_pan::pan::reconcile_finger_pan(state);

    // Simulated edge pan for an ABSOLUTE pointer (winit) sitting in the edge band:
    // it reports nothing while it holds still, so the pan — and the replayed motion
    // that keeps the world point under it current — runs off this frame clock.
    // A no-op unless one is armed.
    compositor_orchestration_seat_pointer_input::extent::tick(state);

    // Frame-end persistence commit — PATH 2 (rim catch-all): a mutation outside
    // `buffer()` flags its world via `mark_world`; here we commit the marked worlds
    // whose debounce is due (immediate, or batched up to 1s), e.g. an overlay world
    // that has stopped being flushed. Buffer transacts commit at their own buffer
    // boundary (`flow::flush`, path 1) and never reach here. Only marked worlds are
    // touched — no per-frame all-world poll; the changed-only diff is in the engine.
    for world_id in compositor_support_system_persist_mark_base::base::due_worlds() {
        if !state.inner.worlds.contains(world_id) {
            continue;
        }
        let world = state.inner.worlds.get(world_id);
        compositor_support_system_persist_flush_base::base::commit_world(
            world_id, world.storage(), &world.systems,
        );
    }

    // DEFERRED (plan): DrawOrder GC of destroyed drawables. The proper form is
    // event-driven — unregister on a drawable-destruction event (DrawOrder.remove
    // at each destroy path) rather than a per-frame live-set scan. Until then a
    // destroyed iced surface leaves a stale order entry, which is harmless
    // (element_of / hit_iced_one return None for it).
    ticked
}

/// The systems tick for one output's frame: publish this output's screen
/// context, then dispatch → `update()` → `draw()` the ACTIVE world and, after
/// it, the KERNEL system host (`WorldManager::kernel`) — the systems that run
/// whatever world is active — into one plan.
///
/// The one tick for the scene and picker passes — each owns a prepare path, and
/// a world that is on screen must tick whichever pass draws it, or a system that
/// only exists to be ticked (the notification pill, the parallax animation)
/// stops the moment the frame plan picks another pass. The LOCK pass deliberately
/// does NOT tick: the lock screen shows no notifications, and not ticking the
/// kernel host is what keeps a queued message waiting for the unlock instead of
/// being consumed unseen.
pub fn world_tick(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) -> Ticked {
    // Per-frame screen context for systems (KernelData). Background systems read
    // physical output size from here (SCREEN) — the former background.shared
    // OUTPUT_SIZE world token is gone.
    {
        let scale = state.size_ctx_all().scale;
        let output = std::sync::Arc::from(state.inner.current_output_key().as_str());
        compositor_orchestration_smithay_data_base::data::update_screen(
            &mut state.inner.kernel,
            compositor_orchestration_smithay_data_base::data::ScreenContext { size, scale, output },
        );
    }
    state.inner.pilot_tick += 1;
    let tick = compositor_support_system_world_frame_base::base::FrameTick {
        index: state.inner.pilot_tick,
        delta: std::time::Duration::ZERO,
    };
    {
        let (worlds, kernel) = (&mut state.inner.worlds, &state.inner.kernel);
        worlds.active_mut().dispatch(kernel);
        worlds.kernel_mut().dispatch(kernel);
    }
    let gpu = state.inner.environment.GPU.clone();
    let mut plan = FramePlan::new();
    {
        // Lend systems the live renderer + window Space via the Platform hatch.
        // SAFETY: platform is dropped at the end of this block; the driver does
        // not touch state.inner.space_state() or the renderer during the calls.
        let mut platform = unsafe {
            compositor_orchestration_draw_platform_base::platform::Platform::new(
                Some(renderer),
                &mut state.inner.space_state_mut().state,
                &gpu,
            )
        };
        let kernel = &state.inner.kernel;
        // Lend the seat (the wayland `Dispatch`) to the update path DISJOINTLY from
        // the world (`&mut state.state` is a different field than `state.inner`), so
        // the navigator system warps the pointer directly via `cx.seat` — no
        // `pending_pointer_warp` round-trip (document/SMITHAY_DECOUPLING.md "P3").
        let seat: &mut dyn std::any::Any = &mut state.state;
        let world = state.inner.worlds.active_mut();
        world.update(kernel, &tick, Some(&mut platform), Some(seat));
        world.draw(kernel, &mut plan, Some(&mut platform));
        let host = state.inner.worlds.kernel_mut();
        host.update(kernel, &tick, Some(&mut platform), Some(seat));
        host.draw(kernel, &mut plan, Some(&mut platform));
    }
    // The kernel host's node, bridged here for every pass: the slide is
    // time-driven, so it needs frames even when nothing else changed.
    let notify = plan.take::<NotifyFrame>();
    if notify.as_ref().is_some_and(|n| n.animating) {
        state.schedule_redraw();
    }
    Ticked { plan, notify }
}

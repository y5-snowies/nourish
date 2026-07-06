use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;

pub fn hooks(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    compositor_y5_window_lifecycle_interface::interface::hook(state, renderer);
    // Promote any disk-restored placeholders (spawn-target world) into visible
    // launcher tiles — needs the renderer, so it can't happen at rehydrate time.
    compositor_y5_placeholder_interface_base::interface::promote_restored(state, renderer);
    compositor_y5_surface_draw_hook::wgpu::hook(state, renderer, size);
    compositor_y5_graphic_capture_interface::interface::per_frame(state, renderer, size);
    // Reconcile the align/distribute selection toolbar against the live selection.
    compositor_y5_select_overlay_interface::interface::per_frame(state, renderer, size);
    // Debug FPS overlay (top-right): measures the composited-frame rate.
    compositor_y5_surface_draw_fps::fps::per_frame(state, renderer, size);
    // Per-frame screen context for systems (KernelData). Background systems read
    // physical output size from here (SCREEN) — the former background.shared
    // OUTPUT_SIZE world token is gone.
    {
        let scale = state.size_ctx_all().scale;
        compositor_orchestration_smithay_data_base::data::update_screen(
            &mut state.inner.kernel,
            compositor_orchestration_smithay_data_base::data::ScreenContext { size, scale },
        );
    }
    state.inner.pilot_tick += 1;
    let tick = compositor_support_system_world_frame_base::base::FrameTick {
        index: state.inner.pilot_tick,
        delta: std::time::Duration::ZERO,
    };
    compositor_orchestration_bus_legacy_base::legacy::drain(state, |l| &mut l.inner.bus);
    {
        let (worlds, kernel) = (&mut state.inner.worlds, &state.inner.kernel);
        worlds.active_mut().dispatch(kernel);
    }
    {
        // Lend systems the live renderer + window Space via the Platform hatch.
        // SAFETY: platform is dropped at the end of this block; the driver does
        // not touch state.inner.space_state() or the renderer during the update call.
        let mut platform = unsafe {
            compositor_orchestration_draw_platform_base::platform::Platform::new(
                Some(renderer),
                &mut state.inner.space_state_mut().state,
            )
        };
        let kernel = &state.inner.kernel;
        // Lend the seat (the wayland `Dispatch`) to the update path DISJOINTLY from
        // the world (`&mut state.state` is a different field than `state.inner`), so
        // the navigator system warps the pointer directly via `cx.seat` — no
        // `pending_pointer_warp` round-trip (document/SMITHAY_DECOUPLING.md "P3").
        let seat: &mut dyn std::any::Any = &mut state.state;
        state
            .inner
            .worlds
            .active_mut()
            .update(kernel, &tick, Some(&mut platform), Some(seat));
    }

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
}

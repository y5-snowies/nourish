//! Per-frame picker pre-step: advance drag-release momentum, push the
//! authoritative transform to the scene, and extract the picker's OWN parallax
//! background (distant/lock-style) to draw behind the sphere.

use smithay::backend::renderer::gles::GlesRenderer;
use compositor_background_two_draw_element::element::ParallaxBackground;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

pub fn tick(state: &mut Loop, renderer: &mut GlesRenderer) -> Option<ParallaxBackground> {
    // Drain the session surface channel; act on picker-panel messages.
    let messages: Vec<_> = {
        let surface = state.inner.surface_mut();
        let mut v = Vec::new();
        while let Ok(m) = surface.surface_message_buffer_channel.1.try_recv() {
            v.push(m);
        }
        v
    };
    for m in messages {
        if let compositor_y5_surface_protocol_base::protocol::SurfaceMessageType::Picker(pm) =
            m.message
        {
            compositor_y5_picker_surface_handle::handle::delegate(state, pm);
        }
    }

    // Momentum / re-face glide + transform push (shared with the overview's
    // embedded globe, so both advance in wall time).
    compositor_y5_picker_command_advance::advance::advance(state);
    ensure_distant_parallax(state, renderer);

    // Extract the picker world's parallax node (mirrors the orchestration scene).
    let mut frame = compositor_support_system_world_frame_base::base::FramePlan::new();
    let mut platform = unsafe {
        compositor_orchestration_draw_platform_base::platform::Platform::new(
            Some(renderer),
            &mut state.inner.space_state_mut().state,
        )
    };
    let kernel = &state.inner.kernel;
    state.inner.worlds.active_mut().draw(kernel, &mut frame, Some(&mut platform));
    drop(platform);
    let bg = frame
        .sorted()
        .into_iter()
        .find_map(|(_, node)| node.downcast::<ParallaxBackground>().ok().map(|b| *b));
    if bg.is_some() {
        state.schedule_redraw_post_vblank();
    }
    // See `ParallaxBackground::bind_overlay` — the picker builds its own plan, so
    // nothing else supplies the pane, refresh or frame serial.
    bg.map(|mut b| {
        let out: std::sync::Arc<str> =
            std::sync::Arc::from(state.inner.current_output_key().as_str());
        if let Some(w) = b.world.filter(|w| state.inner.worlds.contains(*w)) {
            b.bind_frame(compositor_pipeline_world_system_base::base::frame(state.inner.worlds.get(w).storage(), &out));
        }
        b.bind_overlay(
            "picker",
            &out,
            state.inner.current_refresh(),
            state.inner.next_frame_serial(),
        );
        b
    })
}

/// Ensure the picker's OWN parallax instance exists (create it DIRECTLY — the
/// picker's custom render path doesn't drain `TwoSystem`'s buffer, so the normal
/// `update()→SetInstance` never lands) and give it the subtle "distant" look.
fn ensure_distant_parallax(state: &mut Loop, renderer: &mut GlesRenderer) {
    let (w, h) = state.size_ctx_all().screen_size_physical;
    if let Some(two) = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .try_get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT)
    {
        let sel = compositor_model_stats_registry_base::base::background_shader_default();
        let inst = two
            .instance
            .get_or_insert_with(|| {
                // `optimized: false` deliberately: the picker is its own world with
                // its own `Two` slot and no settings UI, so it renders the reference.
                let mut i = ParallaxBackground::new(renderer, (w as f32, h as f32), sel.as_deref(), &[], false);
                // The second place an instance is constructed, and so the second
                // place its world must be stamped — `TwoSystem::buffer` never
                // runs for this slot (see below), so nothing else would. Without
                // it the picker's backdrop shares a worker pane with whatever
                // world is behind it.
                i.world = Some(PICKER_WORLD);
                // The picker fills this slot itself, synchronously, during the
                // render pass — so `TwoSystem`'s rebuild (which only fires on an
                // empty slot) never runs for this world. Without this the picker's
                // full-screen shader rendered inline on the compositor thread
                // whatever `background_triple_buffer` said.
                i.attach_worker();
                i
            });
        // Snap, don't ramp: the picker owns its own entry transition, and the
        // 1s `lock_amount` fade in `Motion::tick` ran as a second, competing
        // background animation on top of it.
        if inst.lock_time.is_none() {
            inst.snap_locked();
        }
        inst.update(); // advance the parallax animation (the buffer Tick won't run)
        inst.pan = (0.0, 0.0);
        inst.zoom = 0.85;
    }
}

//! Per-frame picker pre-step: advance drag-release momentum, push the
//! authoritative transform to the scene, tick the picker world's systems and
//! extract the picker's OWN parallax background (distant/lock-style) to draw
//! behind the sphere, plus the notification pill to draw above it.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Size};
use compositor_background_two_draw_element::element::ParallaxBackground;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_notify_present_base::base::NotifyFrame;
use compositor_y5_picker_system_base::base::{PICKER_MUT, PICKER_WORLD};

pub fn tick(
    state: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> (Option<ParallaxBackground>, Option<NotifyFrame>) {
    compositor_y5_picker_surface_handle::handle::drain(state);

    // Momentum / re-face glide + transform push (shared with the overview's
    // embedded globe, so both advance in wall time).
    compositor_y5_picker_command_advance::advance::advance(state);
    // Before the tick: `TwoSystem::update` only builds an instance for an EMPTY
    // slot, and the picker's must be the distant one.
    ensure_distant_parallax(state, renderer);

    // The same systems tick the orchestration scene runs — the picker world is
    // the active world while it owns the frame, and it is drawn by this pass alone.
    let compositor_orchestration_draw_scene_frame::hooks::Ticked { mut plan, notify } =
        compositor_orchestration_draw_scene_frame::hooks::world_tick(state, renderer, size);
    let bg = plan.take::<ParallaxBackground>();
    if bg.is_some() {
        state.schedule_redraw_post_vblank();
    }
    // See `ParallaxBackground::bind_overlay` — the picker builds its own plan, so
    // nothing else supplies the pane, refresh or frame serial.
    let bg = bg.map(|mut b| {
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
    });
    (bg, notify)
}

/// Ensure the picker's OWN parallax instance exists (create it DIRECTLY, ahead of
/// the world tick — `TwoSystem::update` would otherwise fill the empty slot with
/// the stock instance) and give it the subtle "distant" look.
fn ensure_distant_parallax(state: &mut Loop, renderer: &mut GlesRenderer) {
    // Cloned up front: the instance is built inside a closure that cannot hold a
    // borrow of `state`, and the handle is one `Arc`.
    let formats =
        state.inner.kernel.get(&compositor_kernel_graphic_format_registrar_base::registrar::FORMATS).clone();
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
                i.attach_worker(&formats);
                i
            });
        // Snap, don't ramp: the picker owns its own entry transition, and the
        // 1s `lock_amount` fade in `Motion::tick` ran as a second, competing
        // background animation on top of it.
        if inst.lock_time.is_none() {
            inst.snap_locked();
        }
        // The animation advances through `TwoSystem`'s buffer `Tick`, which the
        // world tick above now delivers for this world too.
        inst.pan = (0.0, 0.0);
        inst.zoom = 0.85;
    }
}

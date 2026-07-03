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

    // Not dragging: play out trackball release momentum, else glide to the target.
    use compositor_y5_picker_three_constant as c;
    use compositor_y5_picker_three_orient::orient;
    if let Some(a) =
        state.inner.worlds.get_mut(PICKER_WORLD).storage_mut().get_mut(&PICKER_MUT).active.as_mut()
        && a.drag.is_none()
    {
        if orient::spinning(a.spin) {
            let (o, s) = orient::momentum(a.orientation, a.spin, c::SPIN_DECAY);
            (a.orientation, a.spin, a.target) = (o, s, o);
        } else {
            a.orientation = orient::approach(a.orientation, a.target, c::APPROACH_RATE);
        }
    }
    compositor_y5_picker_command_base::base::push_transform(state);
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
    bg
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
        let sel = compositor_developer_stats_registry_base::base::background_shader_default();
        let inst = two
            .instance
            .get_or_insert_with(|| {
                ParallaxBackground::new(renderer, (w as f32, h as f32), sel.as_deref(), &[])
            });
        if inst.lock_time.is_none() {
            inst.lock_time = Some(std::time::Instant::now());
        }
        inst.update(); // advance the parallax animation (the buffer Tick won't run)
        inst.pan = (0.0, 0.0);
        inst.zoom = 0.85;
    }
}

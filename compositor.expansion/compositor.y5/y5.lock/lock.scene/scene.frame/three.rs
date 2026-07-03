use smithay::{
    backend::renderer::gles::GlesRenderer,
    utils::{Physical, Point, Size},
};
use compositor_support_bevy_core_compositor_base::BevyRenderElement;
use compositor_orchestration_core_state_base::{Loop, state::CoordinateTrait};
use compositor_monitor_compositor_iced_base::IcedRenderElement;

pub fn scene(
    _loop: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> Vec<BevyRenderElement> {
    let (mut bevy_elements): Vec<BevyRenderElement> = vec![];

    let compositor_orchestration_core_state_base::state::Status::Locked { pending, time, .. } =
        _loop.inner.status
    else {
        abort!();
    };

    if pending {
        return bevy_elements;
    }

    let scale = _loop.size_ctx_all().scale;
    let camera_transform = _loop.inner.camera().transform.clone();
    // The LOCK world owns its bevy registry (prewarmed); render from it.
    if let Some(bevy_registry) = _loop.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT).and_then(|b| b.registry.as_mut()) {
        let transform = compositor_support_bevy_core_compositor_base::Transform {
            zoom: camera_transform.zoom,
            position: Point::new(
                camera_transform.position.x * scale,
                camera_transform.position.y * scale,
            ),
        };
        // Requires gles renderer on every frame. Temporary. should store it instead.
        bevy_elements = bevy_registry
            .render_all(
                &_loop.inner.environment.GPU.as_str(),
                renderer,
                transform,
                size.to_f64(),
                compositor_orchestration_draw_layer_base::base::Layer::LOCK_SCENE.bits(),
            )
            .unwrap_or_default();
    } else {
        // iced_elements = vec![];
        // iced_elements_screen = vec![];
    }

    return bevy_elements;
}

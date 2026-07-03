use smithay::{
    backend::renderer::gles::GlesRenderer,
    utils::{Physical, Point, Size},
};
use compositor_orchestration_core_state_base::{Loop, state::CoordinateTrait};
use compositor_monitor_compositor_iced_base::IcedRenderElement;

pub fn scene(
    _loop: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> Vec<IcedRenderElement> {
    // Right now explicit. Later on render scene when it accepts Gles only.
    // let (iced_elements) = scene_gles::scene_gles(state, gles_renderer);

    let (mut iced_elements): Vec<IcedRenderElement> = vec![];

    let compositor_orchestration_core_state_base::state::Status::Locked { pending, time, .. } =
        _loop.inner.status
    else {
        abort!();
    };

    if pending {
        return iced_elements;
    }

    let scale = _loop.size_ctx_all().scale;
    let camera_transform = _loop.inner.camera().transform.clone();
    let gpu = _loop.inner.environment.GPU.clone();
    let mut wants_frame = false;
    if let Some(ref mut iced) = _loop.inner.surface_mut().registry {
        let transform = compositor_monitor_compositor_iced_base::Transform {
            zoom: camera_transform.zoom,
            position: Point::new(
                camera_transform.position.x * scale,
                camera_transform.position.y * scale,
            ),
        };
        // Requires gles renderer on every frame. Temporary. should store it instead.
        iced_elements = iced
            .render_all(
                &gpu.as_str(),
                renderer,
                transform,
                size.to_f64(),
                compositor_orchestration_draw_layer_base::base::Layer::LOCK_SCENE.bits(),
            )
            .unwrap_or_default();
        wants_frame = iced.wants_frame();
    } else {
        // iced_elements = vec![];
        // iced_elements_screen = vec![];
    }

    // Keep the vblank cycle alive while iced is animating in the lock scene.
    if wants_frame {
        _loop.schedule_redraw_post_vblank();
    }

    return iced_elements;
}

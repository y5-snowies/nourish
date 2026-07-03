use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_monitor_compositor_iced_base::{IcedRenderElement, IcedSpace};

/// Returns `(world, screen, dim)` iced render-element vecs:
/// - `world`  — `IcedSpace::World`, SCENE layer (drawn below windows);
/// - `screen` — `IcedSpace::Screen`, SCENE layer (drawn above windows);
/// - `dim`    — the `CAPTURE_DIM` layer (the capture backdrop), drawn in its
///   own pass between windows and the world/background layers.
pub fn scene(
    _loop: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> (
    Vec<IcedRenderElement>,
    Vec<IcedRenderElement>,
    Vec<IcedRenderElement>,
) {
    // Right now explicit. Later on render scene when it accepts Gles only.
    // let (iced_elements) = scene_gles::scene_gles(state, gles_renderer);

    let (mut iced_elements, mut iced_elements_screen, mut iced_elements_dim): (
        Vec<IcedRenderElement>,
        Vec<IcedRenderElement>,
        Vec<IcedRenderElement>,
    ) = (vec![], vec![], vec![]);

    let scale = _loop.size_ctx_all().scale;
    // Hoist the camera + GPU reads before the surface-registry borrow: surface_mut()
    // borrows the whole Orchestrator, so other inner fields must be read first.
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
        let res = iced
            .render_all(
                &gpu.as_str(),
                renderer,
                transform,
                size.to_f64(),
                compositor_orchestration_draw_layer_base::base::Layer::SCENE.bits(),
            )
            .unwrap_or_default();

        for item in res {
            match item.space {
                IcedSpace::World => {
                    iced_elements.push(item);
                }
                IcedSpace::Screen => {
                    iced_elements_screen.push(item);
                }
            }
        }

        // Capture-dim pass: rendered separately so the scene builder can place
        // it BELOW windows (its own z-slot), unlike the screen-space SCENE pass
        // which sits above them.
        let dim = iced
            .render_all(
                &gpu.as_str(),
                renderer,
                transform,
                size.to_f64(),
                compositor_orchestration_draw_layer_base::base::Layer::CAPTURE_DIM.bits(),
            )
            .unwrap_or_default();
        iced_elements_dim.extend(dim);

        // Any instance still dirty or mid-animation wants another frame.
        wants_frame = iced.wants_frame();
    }

    // Keep the vblank cycle alive while iced is animating, so time-based
    // animations advance frame-to-frame (mirrors the parallax background's
    // self-scheduling). Done after the registry borrow is released.
    if wants_frame {
        _loop.schedule_redraw_post_vblank();
    }

    return (iced_elements, iced_elements_screen, iced_elements_dim);
}

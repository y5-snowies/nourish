//! The picker render pass (bevy sphere + parallax + iced panel + pointer),
//! reusing the orchestration `Scene`/`Plan`/`SceneElement` pipeline.
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{ImportAll, ImportDma, ImportMem, Renderer, Texture};
use smithay::utils::{Physical, Point, Size};

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_orchestration_draw_node_base::node::{DrawNode, Plan};
use compositor_orchestration_draw_scene_frame::scene::Scene;
use compositor_support_bevy_core_compositor_base::BevyRenderElement;
use compositor_support_system_world_frame_base::base as layer;

/// GLES-built elements carried from `prepare()` into `scene()`: the bevy sphere,
/// the picker's own parallax background, and the bottom-right details panel.
pub struct PickerPrepared {
    pub bevy: Vec<BevyRenderElement>,
    pub background_two:
        Option<compositor_background_two_draw_element::element::ParallaxBackground>,
    pub surfaces: Vec<compositor_monitor_compositor_iced_base::IcedRenderElement>,
}

/// Active-monitor-only gate. The picker overlay (sphere + details panel + pointer)
/// renders ONLY on the active output; other outputs keep just the parallax, so the
/// sphere/panel aren't stretched/duplicated on differently-sized monitors and their
/// baked size stays stable. `render_output == None` = winit/single-output pass →
/// active (that path is unchanged). Mirrors the overview overlay's convention.
fn on_active_output(s: &Loop) -> bool {
    s.inner
        .render_output
        .as_ref()
        .is_none_or(|k| *k == s.inner.active_output_key())
}

/// GLES preparation: render the picker bevy instance (tagged `PICKER_SCENE`).
pub fn prepare(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) -> PickerPrepared {
    use compositor_y5_picker_system_base::base::PICKER_WORLD;

    // Per-frame pre-step: momentum, transform push, parallax extraction. Runs on
    // every output so the parallax fills each monitor behind the picker.
    let background_two = compositor_y5_picker_scene_tick::tick::tick(state, renderer);

    // Non-active output: parallax only. Skip the (size-baked) bevy sphere and iced
    // details panel so they render exclusively on the active monitor.
    if !on_active_output(state) {
        return PickerPrepared { bevy: vec![], background_two, surfaces: vec![] };
    }

    let gpu = state.inner.environment.GPU.clone();
    // The sphere is baked at the active output's size in `scene.create` (the only
    // output it renders on), so `size` here matches and no live resize is needed —
    // resizing an open picker raced its first render and blanked the bevy.
    let bevy = if let Some(reg) = state
        .inner
        .worlds
        .get_mut(PICKER_WORLD)
        .storage_mut()
        .try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT)
        .and_then(|b| b.registry.as_mut())
    {
        let transform = compositor_support_bevy_core_compositor_base::Transform {
            zoom: 1.0,
            position: Point::new(0.0, 0.0),
        };
        reg.render_all(&gpu, renderer, transform, size.to_f64(), Layer::PICKER_SCENE.bits())
            .unwrap_or_default()
    } else {
        vec![]
    };

    let mut iced_wants_frame = false;
    let surfaces = if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let t = compositor_monitor_compositor_iced_base::Transform {
            zoom: 1.0,
            position: Point::new(0.0, 0.0),
        };
        let els = reg
            .render_all(&gpu, renderer, t, size.to_f64(), Layer::PICKER_SCENE.bits())
            .unwrap_or_default();
        iced_wants_frame = reg.wants_frame();
        els
    } else {
        vec![]
    };

    // Keep the vblank cycle alive while iced is animating in the picker scene.
    if iced_wants_frame {
        state.schedule_redraw_post_vblank();
    }

    PickerPrepared { bevy, background_two, surfaces }
}

/// Lower the bevy elements (+ pointer) into the renderer-agnostic `Scene`.
pub fn scene<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    prepared: PickerPrepared,
) -> Scene<R>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let mut plan: Plan<R> = Plan::new();
    // Active monitor only: fade overlay, pointer, details panel, and the sphere.
    // Other outputs draw the parallax alone (below), so the picker UI never lands
    // on a monitor the user isn't on.
    if on_active_output(state) {
        // Entry fade: a black overlay (above the scene) that clears over FADE_SECS.
        if let Some(solid) = compositor_y5_picker_scene_fade::fade::overlay(state, size) {
            plan.push(layer::POINTER, DrawNode::Solid(solid));
        }
        // Pointer on top, then the sphere.
        let pointer =
            compositor_orchestration_seat_pointer_draw::scene::element(state, renderer, size);
        plan.extend(layer::POINTER, pointer.into_iter().map(DrawNode::Pointer));
        // Details panel above the sphere (but below the pointer).
        plan.extend(layer::ICED_SCREEN, prepared.surfaces.into_iter().map(DrawNode::Iced));
        plan.extend(layer::WORLD_3D, prepared.bevy.into_iter().map(DrawNode::Background3D));
    }
    if let Some(bg) = prepared.background_two {
        plan.push(layer::BACKGROUND, DrawNode::Background2D(bg));
    }
    let (elements, meta) = plan.lower(renderer);
    Scene {
        Element: elements,
        meta,
        visible_window: vec![],
    }
}

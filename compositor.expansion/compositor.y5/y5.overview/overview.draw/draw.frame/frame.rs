//! Overview frame integration, encapsulated. The orchestration scene calls
//! `prepare` (GLES phase) and `band` (renderer-agnostic phase); the overview owns
//! everything else (backdrop capture/blur, grid, globe), so the rim stays thin.

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{ImportAll, ImportDma, ImportMem, Renderer, Texture};
use smithay::desktop::Window;
use smithay::utils::{Physical, Point, Rectangle, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use compositor_orchestration_draw_node_base::node::{DrawNode, Plan};
use compositor_orchestration_draw_scene_element::element::PreImported;
use compositor_support_bevy_core_compositor_base::BevyRenderElement;
use compositor_support_system_world_frame_base::base as layer;
use compositor_y5_overview_state_base::base::Tab;

fn solid(rect: Rectangle<i32, Physical>, color: [f32; 4]) -> SolidColorRenderElement {
    SolidColorRenderElement::new(Id::new(), rect, CommitCounter::default(), color, Kind::Unspecified)
}

/// The overview overlay is active-monitor-only. `prepare`/`band` run once PER OUTPUT
/// in the render loop, so act only on the active output's pass — else the embedded
/// globe (keyed on one `GLOBE_SIZE`) thrashes between differently-sized monitors and
/// never renders. `render_output == None` = winit/single pass → act.
fn on_active_output(s: &Loop) -> bool {
    s.inner.render_output.as_ref().is_none_or(|k| *k == s.inner.active_output_key())
}

/// GLES phase: advance the freeze-backdrop capture, and on the World tab render
/// the embedded picker globe (else tear it down). Returns the globe's bevy
/// elements for `band`.
pub fn prepare(state: &mut Loop, gles: &mut GlesRenderer, size: Size<i32, Physical>) -> Vec<BevyRenderElement> {
    // Active monitor only (keeps the embedded globe's GLOBE_SIZE stable).
    if !on_active_output(state) { return Vec::new(); }
    compositor_y5_overview_draw_backdrop::backdrop::arm(state, gles, size);
    // Settings tab: reconcile the embedded settings iced surface (no-op off-tab).
    compositor_y5_overview_draw_settings::settings::per_frame(state, gles, size);
    // Menu-bar clock + Display FPS (throttled) + keep the menu bar output-width-sized.
    compositor_y5_overview_draw_status::status::per_frame(state, size);
    if state.inner.overview().visible && state.inner.overview().overlay_ready() && state.inner.overview().is_world() {
        compositor_y5_overview_draw_world::world::prepare_world(state, gles, size)
    } else {
        compositor_y5_picker_interface_embed::embed::embed_close(state);
        Vec::new()
    }
}

/// CONTENT band: when the overlay is shown, push the backdrop (frozen blurred
/// snapshot + scrim, or a dim fallback) and the active tab's content (Layout
/// grid / World globe / blank Settings), returning the windows drawn (for frame
/// callbacks). `None` when the overlay isn't shown — the caller draws the canvas.
pub fn band<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    plan: &mut Plan<R>,
    world: Vec<BevyRenderElement>,
) -> Option<Vec<Window>>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    if !(state.inner.overview().visible && state.inner.overview().overlay_ready()) {
        return None;
    }
    // Active monitor only — else the caller draws that output's normal canvas.
    if !on_active_output(state) { return None; }
    let full = Rectangle::new(Point::from((0, 0)), size);
    let mut have_snapshot = false;
    if let Some(dmabuf) = compositor_y5_overview_draw_backdrop::backdrop::snapshot_dmabuf(state) {
        if let Ok(texture) = renderer.import_dmabuf(&dmabuf, None) {
            plan.push(layer::CAPTURE_DIM, DrawNode::Solid(solid(full, [0.0, 0.0, 0.0, 0.45])));
            plan.push(layer::CAPTURE_DIM, DrawNode::Texture(PreImported {
                texture,
                location: Point::from((0, 0)),
                size,
                world_zoom: 1.0,
                id: Id::new(),
                commit: CommitCounter::default(),
            }));
            have_snapshot = true;
        }
    }
    if !have_snapshot {
        plan.push(layer::CAPTURE_DIM, DrawNode::Solid(solid(full, [0.02, 0.02, 0.03, 0.92])));
    }
    Some(match state.inner.overview().tab {
        Tab::Layout => {
            let (grid, windows) = compositor_y5_overview_draw_scene::scene::scene(state, renderer, size);
            for e in grid {
                plan.push(layer::CANVAS, DrawNode::Canvas(e));
            }
            windows
        }
        Tab::World => {
            for e in world {
                plan.push(layer::CANVAS, DrawNode::Background3D(e));
            }
            Vec::new()
        }
        Tab::Settings => Vec::new(),
    })
}

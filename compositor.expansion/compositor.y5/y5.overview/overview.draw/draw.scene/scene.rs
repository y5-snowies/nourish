//! Overview Layout-tab render: fit each placed window live into its grid cell.
//! Placement/order/scroll come from [`compositor_y5_overview_draw_plan`].
//! Front-most first. Purely presentational — a click (handled in the seat) closes
//! the overlay and views the window; nothing is drawn per-selection here.

use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::desktop::Window;
use smithay::utils::{Physical, Size};
use smithay::wayland::seat::WaylandFocus;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_overview_draw_fit::fit::fit_window;
use compositor_y5_overview_draw_plan::plan::plan;
use compositor_y5_canvas_draw_element::element::Element as CanvasElement;

pub fn scene<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
) -> (Vec<CanvasElement<R>>, Vec<Window>)
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let placed = plan(state, size);
    if placed.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let scale = state.size_ctx_all().scale;

    let mut elements: Vec<CanvasElement<R>> = Vec::new();
    let mut drawn: Vec<Window> = Vec::new();
    for (window, rect) in &placed {
        let Some(root) = window.wl_surface().map(|c| c.into_owned()) else { continue };
        let fitted = fit_window(renderer, &root, window.geometry(), *rect, scale, size);
        if fitted.is_empty() {
            continue;
        }
        drawn.push(window.clone());
        elements.extend(fitted);
    }
    (elements, drawn)
}

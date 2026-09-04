//! Overview grid placement: order windows spatially (top-to-bottom, then
//! left-to-right by world position), size + lay out the grid, apply/clamp the
//! vertical scroll, and record cell rects into the slot for click hit-testing.
//! Returns the placed `(window, screen-rect)` pairs for the scene to render.

use smithay::desktop::Window;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use smithay::wayland::seat::WaylandFocus;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_overview_draw_grid::grid::{self, GridParams};
use compositor_y5_overview_state_base::base::MENU_BAR_HEIGHT;
use compositor_y5_window_interface_record::window::LoopWindow;

const GRID_GAP: i32 = 24;
const GRID_MARGIN: i32 = 40;

pub fn plan(state: &mut Loop, size: Size<i32, Physical>) -> Vec<(Window, Rectangle<i32, Physical>)> {
    let mut windows: Vec<(Window, Point<i32, Logical>)> = state
        .inner
        .space_state()
        .state
        .elements()
        .filter(|w| compositor_support_smithay_state_window_ident::ident::is_drawn(w))
        .map(|w| {
            let loc = state.inner.space_state().state.element_location(w).unwrap_or_default();
            (w.clone(), loc)
        })
        .collect();
    if windows.is_empty() {
        state.inner.overview_mut().cells.clear();
        return Vec::new();
    }
    // Reading order: higher windows (smaller y) first, then left-to-right.
    windows.sort_by_key(|(_, loc)| (loc.y, loc.x));

    let aspects: Vec<f64> = windows
        .iter()
        .map(|(w, _)| {
            let g = w.geometry().size;
            g.w as f64 / g.h as f64
        })
        .collect();

    let area = Rectangle::new(
        Point::from((0, MENU_BAR_HEIGHT)),
        Size::from((size.w, (size.h - MENU_BAR_HEIGHT).max(1))),
    );
    let inner_h = (area.size.h - 2 * GRID_MARGIN).max(1);
    let inner_w = (area.size.w - 2 * GRID_MARGIN).max(1);
    // DON'T fill the screen. Size cells by the windows' REALIZED OCCUPANCY — total
    // grid footprint (area = cell_h² · Σaspect, so wide windows count for more than
    // narrow ones) — targeting a fraction of the viewport AREA, solved for cell_h.
    // Shrinks smoothly as windows get more/wider (not a coarse count tier): a lone
    // window is a card, not the whole 4K screen, and the grid overflows + scrolls
    // (clamped below) when it genuinely doesn't fit. Width-bounded + height-capped.
    const TARGET_OCCUPANCY: f64 = 0.30;
    let max_aspect = aspects.iter().cloned().fold(0.05_f64, f64::max);
    let sum_aspect: f64 = aspects.iter().map(|a| a.max(0.05)).sum();
    let by_area = (TARGET_OCCUPANCY * inner_w as f64 * inner_h as f64 / sum_aspect).sqrt() as i32;
    let by_width = (inner_w as f64 / max_aspect) as i32;
    // Hard cap so no single cell (e.g. a lone square) dominates the viewport height.
    let by_height = (inner_h as f64 * 0.60) as i32;
    let cell_h = by_area.min(by_width).min(by_height).clamp(80, inner_h);
    let (cells, content_h) = grid::layout(
        area,
        &aspects,
        GridParams { gap: GRID_GAP, cell_height: cell_h, margin: GRID_MARGIN, max_cols: 5 },
    );

    // Clamp scroll to the overflow; write it back so the axis handler accumulates
    // against the real range.
    let max_scroll = (content_h - inner_h).max(0) as f64;
    let scroll = state.inner.overview().scroll.clamp(0.0, max_scroll);
    state.inner.overview_mut().scroll = scroll;
    let scroll = scroll.round() as i32;

    let mut placed = Vec::with_capacity(cells.len());
    let mut recorded = Vec::with_capacity(cells.len());
    for cell in &cells {
        let (window, _) = &windows[cell.index];
        let rect = Rectangle::new(
            Point::from((cell.rect.loc.x, cell.rect.loc.y - scroll)),
            cell.rect.size,
        );
        if let Some(uuid) = window.uuid() {
            recorded.push((uuid, rect));
        }
        placed.push((window.clone(), rect));
    }
    state.inner.overview_mut().cells = recorded;
    placed
}

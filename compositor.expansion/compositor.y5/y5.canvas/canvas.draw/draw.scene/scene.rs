use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::desktop::Window;
use smithay::utils::{Physical, Point, Size};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_monitor_compositor_iced_base::{HandleId, IcedRenderElement, Transform as IcedTransform};
use compositor_y5_canvas_draw_element::element::Element;
use compositor_y5_window_interface_record::window::LoopWindow;

/// One drawable in the content band: a canvas element (window / select-box /
/// cursor) or an iced surface. Windows and world iced interleave here by the
/// renderer-agnostic DrawOrder ("everything interleaves").
pub enum ContentItem<R: Renderer> {
    Canvas(Element<R>),
    Iced(IcedRenderElement),
}

fn placed(window: &Window) -> bool {
    window.user_data().get::<compositor_support_smithay_state_compositor_dispatch::wire::WindowPlacedMarker>().is_some()
}

pub fn scene<R>(state: &mut Loop, renderer: &mut R, size: Size<i32, Physical>) -> (Vec<ContentItem<R>>, Vec<Window>)
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let canvas_context = context(state, renderer, size);
    let mut content: Vec<ContentItem<R>> = Vec::new();
    let mut visible_windows = Vec::new();

    // Select box overlays the content (front-most within the band).
    for e in compositor_y5_select_box_base::select_box::select_box(state, renderer, size, &canvas_context) {
        content.push(ContentItem::Canvas(Element::SolidBox(e)));
    }

    // Interleave windows + world iced by the DrawOrder authority (topmost-first).
    let order = state.inner.drawable_order();
    let by_uuid: HashMap<Uuid, Window> = state.inner.space_state().state
        .elements().filter_map(|w| w.uuid().map(|u| (u, w.clone()))).collect();
    let ordered: HashSet<Uuid> = order.iter().copied().collect();
    // iced camera transform (mirrors the surface scene): world items pan/zoom.
    let scale = state.viewport_context().scale;
    let cam = state.inner.camera().transform.clone();
    let iced_transform = IcedTransform { zoom: cam.zoom, position: Point::new(cam.position.x * scale, cam.position.y * scale) };
    let size_f64 = size.to_f64();

    let mut draw_window = |state: &mut Loop, renderer: &mut R, window: &Window, content: &mut Vec<ContentItem<R>>, visible: &mut Vec<Window>| {
        if !placed(window) { return; }
        let (elems, vis) = compositor_y5_window_draw_frame::scene::scene(state, renderer, size, window, &canvas_context);
        if vis { visible.push(window.clone()); }
        for e in elems { content.push(ContentItem::Canvas(Element::Window(e))); }
    };

    for uuid in &order {
        if let Some(window) = by_uuid.get(uuid).cloned() {
            draw_window(state, renderer, &window, &mut content, &mut visible_windows);
        } else if let Some(elem) = state.inner.surface().registry.as_ref().and_then(|r| r.element_of(HandleId(uuid.as_u128() as u64), &iced_transform, size_f64)) {
            content.push(ContentItem::Iced(elem));
        }
    }
    // Defensive: any placed window not in the order draws at the bottom.
    let leftovers: Vec<Window> = by_uuid.values().filter(|w| w.uuid().map(|u| !ordered.contains(&u)).unwrap_or(true)).cloned().collect();
    for window in leftovers {
        draw_window(state, renderer, &window, &mut content, &mut visible_windows);
    }

    // Canvas cursor: drawn only on the pane under the physical cursor (the
    // `pointer` slot) — it would otherwise appear once per pane. Outside the
    // per-region loop (no render target) it always draws.
    let cursor_here = state
        .inner
        .render_target
        .map_or(true, |rt| rt.slot == state.inner.viewports().pointer);
    if cursor_here {
        for e in compositor_y5_canvas_cursor_element::scene::scene(state, renderer, size, &canvas_context) {
            content.push(ContentItem::Canvas(Element::SolidBox(e)));
        }
    }

    (content, visible_windows)
}

pub use compositor_y5_canvas_draw_viewport::viewport::context;

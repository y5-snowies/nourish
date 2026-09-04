use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Physical, Point, Size};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_monitor_compositor_iced_base::{
    HandleId, IcedRenderElement, IcedSpace, Transform as IcedTransform,
};
use compositor_y5_canvas_draw_element::element::Element;
use compositor_y5_window_draw_occlude::occlude::{Occluders, Visible};
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_pipeline_abi_clock_base::base as clock;
use compositor_pipeline_abi_descriptor_base::base::{self as d, Times};
use compositor_support_smithay_state_window_find::find;

/// One drawable in the content band: a canvas element (window / select-box /
/// cursor) or an iced surface. Windows and world iced interleave here by the
/// renderer-agnostic DrawOrder ("everything interleaves").
pub enum ContentItem<R: Renderer> {
    /// `flags` are the `window.descriptor` bits of the window this element belongs
    /// to and `times` its timestamp row; zero and "never" for the canvas furniture
    /// (select box, cursor solid) that belongs to no window. Attached HERE because
    /// this is the last point at which the window is still in hand — see
    /// [`window_flags`].
    Canvas { elem: Element<R>, flags: u32, times: Times },
    Iced(IcedRenderElement),
}

/// The descriptor bits for one window, read from the compositor's own authorities.
///
/// `focus` is the keyboard-focus surface, resolved once for the frame by the
/// caller: it is a seat-wide fact, and asking per window would walk the space for
/// every window on screen.
///
/// Every bit is a value the compositor already maintains for its own reasons —
/// nothing here introduces state to feed a shader, which is what keeps a
/// descriptor from becoming a second source of truth that can disagree with the
/// first.
fn window_flags(
    window: &Window,
    focus: Option<&WlSurface>,
    dragged: &[Window],
    topmost: bool,
    selected: bool,
    primary: bool,
) -> u32 {
    let activated = focus.is_some_and(|f| find::is_surface(window, f));
    d::when(d::ACTIVATED, activated)
        | d::when(d::TOPMOST, topmost)
        | d::when(d::FULLSCREEN, window.is_fullscreen())
        | d::when(
            d::RESIZING,
            compositor_y5_camera_transform_translate::slot::resize_stretching(
                window,
                window.geometry().size,
            ),
        )
        | d::when(d::MOVING, dragged.contains(window))
        | d::when(d::SELECTED, selected)
        // Always implies SELECTED — `Select::Primary` is a member of `Selection`,
        // so a bundle may test either bit without checking the other.
        | d::when(d::PRIMARY, primary)
}

/// The windows an interactive MOVE grab is currently carrying.
///
/// Resolved once for the frame, like `focus`: the grab is one piece of seat-wide
/// state, and a move carries the whole selection, so this is a list rather than a
/// per-window question.
///
/// Only `Moving` — a `Scaling` grab is a resize, and that already has its own
/// authority in `slot::resize_stretching`. Only windows: a placeholder being
/// dragged is not a client window and never reaches the descriptor lane.
fn dragged_windows(state: &Loop) -> Vec<Window> {
    use compositor_y5_canvas_input_state::state::{ActiveOption, ActiveTransformCandidate, CanvasGrab};
    match &state.inner.canvas().Grab {
        CanvasGrab::Active(ActiveOption::Moving {
            candidates: ActiveTransformCandidate::Window(list),
            ..
        }) => list.iter().map(|(w, _)| w.clone()).collect(),
        _ => Vec::new(),
    }
}

/// The WORLD-space iced surface for `uuid`, if the registry still has one.
///
/// World only. A SCREEN surface reaching this band is drawn TWICE — here, and
/// again in its own band. The copies coincided exactly until a bundle's pointer
/// warp curved one of them and the desktop grew a second launcher. Fixed at the
/// source too (`native_press`), but this loop walks a PERSISTED order and must not
/// assume every id in it belongs to this band.
fn world_iced(
    state: &Loop,
    uuid: &Uuid,
    transform: &IcedTransform,
    size: Size<f64, Physical>,
) -> Option<IcedRenderElement> {
    let id = HandleId(uuid.as_u128() as u64);
    let registry = state.inner.surface().registry.as_ref()?;
    matches!(registry.space_of(id)?, IcedSpace::World)
        .then(|| registry.element_of(id, transform, size))?
}

fn placed(window: &Window) -> bool {
    window.user_data().get::<compositor_support_smithay_state_compositor_dispatch::wire::WindowPlacedMarker>().is_some()
}

pub fn scene<R>(state: &mut Loop, renderer: &mut R, size: Size<i32, Physical>) -> (Vec<ContentItem<R>>, Visible)
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let canvas_context = context(state, renderer, size);
    let mut content: Vec<ContentItem<R>> = Vec::new();
    let mut visible_windows = Visible::default();
    // One accumulator per pane — an opaque rect only hides what is drawn into
    // the same viewport. Filled as the DrawOrder loop below walks front to back,
    // so each window is tested against everything already stacked over it.
    let mut occluders = Occluders::new();

    // Select box overlays the content (front-most within the band).
    for e in compositor_y5_select_box_base::select_box::select_box(state, renderer, size, &canvas_context) {
        content.push(ContentItem::Canvas { elem: Element::SolidBox(e), flags: 0, times: d::no_times(clock::NEVER) });
    }
    // The persistent selection frame (bounding rect + resize/move handles) overlays
    // too, for the armed Select tool / an in-progress select transform.
    for e in compositor_y5_select_box_base::select_box::select_frame(state, renderer, size, &canvas_context) {
        content.push(ContentItem::Canvas { elem: Element::SolidBox(e), flags: 0, times: d::no_times(clock::NEVER) });
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

    // Keyboard focus is a seat-wide fact, so it is resolved once here rather than
    // per window: `window_flags` compares against it.
    let focus: Option<WlSurface> =
        state.state.seat.seat.get_keyboard().and_then(|kb| kb.current_focus());
    let dragged = dragged_windows(state);
    // The canvas selection, resolved ONCE for the frame like `focus` and
    // `dragged`: it is one piece of per-world state and every window compares
    // against the same list.
    let selection: Vec<Window> =
        state.inner.select().Selection.iter().map(|w| (**w).clone()).collect();
    // The anchor of that selection, resolved on the same terms. `None` for a
    // selection of one — `Select::set` clears it — so a lone selected window
    // carries `SELECTED` without `PRIMARY`.
    let primary: Option<Window> = state.inner.select().Primary.as_ref().map(|w| (**w).clone());

    // TOPMOST is the first window that actually DRAWS, which is neither the first
    // id in the order (world iced interleaves and may sit in front) nor the first
    // window in it (an unplaced or culled window contributes nothing to look at).
    // Latched here, in the one place every window goes through, so the two callers
    // below cannot disagree about which of them saw it first.
    // Pane-independent frame identity, so `observe` can fold each window's edges
    // in exactly once however many panes draw it. `top_taken` below is per PANE
    // and deliberately stays that way — it decides what this pane shades, not
    // what the window's history is.
    let frame_id = state.inner.pilot_tick;
    // Whether the drawn world's bundle composites windows itself. ONCE per frame,
    // not once per window: it is a fact about the world's bundle, and with no
    // bundle it is a single token read instead of one per window.
    //
    // The SPAWN TARGET's facts, with no picker branch — windows live in the spawn
    // target's space, and every overlay that puts up a different picture already
    // sets `suppressed`, which makes `covers` false on its own.
    // ONE token read for both answers: whether a bundle is running at all, and
    // whether it composites windows itself.
    let (active, bundle_owned) = {
        let w = state.inner.worlds.spawn_target();
        match state.inner.worlds.contains(w) {
            false => (false, false),
            true => {
                let f = compositor_pipeline_world_system_base::base::facts(
                    state.inner.worlds.get(w).storage(),
                );
                (f.active(), f.covers(true))
            }
        }
    };
    let mut top_taken = false;
    let mut draw_window = |state: &mut Loop, renderer: &mut R, window: &Window, content: &mut Vec<ContentItem<R>>, visible: &mut Visible, occ: &mut Occluders| {
        if !placed(window) { return; }
        let (elems, drawn) = compositor_y5_window_draw_frame::scene::scene(state, renderer, size, window, &canvas_context, occ, bundle_owned);
        visible.note(window, &drawn);
        occ.extend(&drawn.opaque);
        if elems.is_empty() { return; }
        let topmost = !std::mem::replace(&mut top_taken, true);
        // One value for the whole window: its surface tree and its decorations are
        // several entries in the shader's array and they must agree about which
        // window they belong to.
        // One resolution per window per frame, against the selection list the
        // frame already holds — the same shape as `dragged` above.
        let selected = selection.contains(window);
        let is_primary = primary.as_ref() == Some(window);
        let flags = window_flags(window, focus.as_ref(), &dragged, topmost, selected, is_primary);
        // Observed unconditionally, not behind the `window_times` requirement. The
        // work is a lock and a handful of compares per window per frame; gating it
        // would mean a bundle selected at runtime sees every window that was
        // already open report as having opened at the moment of selection, and the
        // whole desktop would play its open animation at once.
        let times = compositor_y5_window_draw_moment::moment::observe(
            window,
            flags & d::ACTIVATED != 0,
            topmost,
            flags & d::RESIZING != 0,
            flags & d::MOVING != 0,
            selected,
            active,
            frame_id,
        );
        for e in elems { content.push(ContentItem::Canvas { elem: Element::Window(e), flags, times }); }
    };

    for uuid in &order {
        if let Some(window) = by_uuid.get(uuid).cloned() {
            draw_window(state, renderer, &window, &mut content, &mut visible_windows, &mut occluders);
            continue;
        }
        let id = HandleId(uuid.as_u128() as u64);
        // SCREEN ids reach this order too (see `world_iced`), and an id the
        // registry no longer knows is stale. Skip both.
        if let Some(IcedSpace::World) = state.inner.surface().registry.as_ref().and_then(|r| r.space_of(id)) {
            if let Some(elem) = world_iced(state, uuid, &iced_transform, size_f64) {
                content.push(ContentItem::Iced(elem));
            }
        }
    }
    // Defensive: any placed window not in the order draws at the bottom.
    let leftovers: Vec<Window> = by_uuid.values().filter(|w| w.uuid().map(|u| !ordered.contains(&u)).unwrap_or(true)).cloned().collect();
    for window in leftovers {
        draw_window(state, renderer, &window, &mut content, &mut visible_windows, &mut occluders);
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
            content.push(ContentItem::Canvas { elem: Element::SolidBox(e), flags: 0, times: d::no_times(clock::NEVER) });
        }
    }

    (content, visible_windows)
}

pub use compositor_y5_canvas_draw_viewport::viewport::context;

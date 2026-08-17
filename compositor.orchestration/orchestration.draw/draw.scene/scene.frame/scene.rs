use crate::layershell::layershell;
use crate::{buffers, hooks};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{AsRenderElements, Id, Kind, RenderElement};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{ImportAll, ImportDma, ImportMem, Renderer, Texture};
use smithay::desktop::space::SpaceRenderElements;
use smithay::desktop::{Window, layer_map_for_output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Scale, Size};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::wlr_layer::Layer as WlrLayer;
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use compositor_orchestration_draw_scene_element::element::SceneElement;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_draw_node_base::node::{DrawNode, Plan};
use compositor_support_system_world_frame_base::base as layer;

pub struct Scene<R: Renderer> {
    pub Element: Vec<SceneElement<R>>,
    /// Lockstep with `Element`: per-element metadata (space, …). Consumed by the
    /// native Vulkan path (wrapped into `VkOutput`) to restrict AA to world
    /// content; ignored by the GLES path.
    pub meta: Vec<compositor_orchestration_draw_dispatch_frame::ElementMeta>,
    pub visible_window: Vec<Window>,
}

thread_local! {
    /// Last fractional scale emitted per surface — the dedup so `update_fractional`
    /// only re-sends `wp_fractional_scale` when a window's best-resolution scale
    /// actually changes, not every frame. Keyed by the surface's protocol id;
    /// `Published` distinguishes a real scale from the idle sentinel explicitly.
    static FRAC_SENT: std::cell::RefCell<
        std::collections::HashMap<
            smithay::reexports::wayland_server::backend::ObjectId,
            compositor_support_smithay_state_fractional_dispatch::Published,
        >,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Per-window fractional scale, best-resolution across ALL outputs. A window may be
/// visible in several viewports spread over several monitors at different zooms; its
/// preferred scale follows the HIGHEST-zoom (sharpest) one. Derived from the live
/// per-output view state (`output_views`: each slot's camera zoom + its `on_pane_grace`
/// window set), so it's independent of which output is mid-render — and emitted only
/// on change (via `FRAC_SENT`), which is what stops the per-output flip-flop from
/// re-sending the scale to clients every frame. `emit_best_per_surface` then
/// debounces whatever is left pending, so a zoom ease or a drift pan publishes once
/// it settles rather than at every lattice crossing.
fn update_fractional(state: &mut Loop) {
    use smithay::reexports::wayland_server::Resource;
    // uuid → surface for currently-mapped windows (the `on_pane_grace` sets store uuids).
    let uuid_surface: std::collections::HashMap<uuid::Uuid, WlSurface> = state
        .inner
        .space_state()
        .state
        .elements()
        .filter_map(|w| Some((w.uuid()?, w.wl_surface()?.into_owned())))
        .collect();
    // Highest zoom per surface across every output's viewports.
    let mut best: std::collections::HashMap<
        smithay::reexports::wayland_server::backend::ObjectId,
        (f64, WlSurface),
    > = std::collections::HashMap::new();
    let mut max_zoom: Option<f64> = None;
    for vps in state.inner.output_views().map.values() {
        for (slot, uuids) in &vps.on_pane_grace {
            let zoom = vps.camera_of(*slot).map(|c| c.transform.zoom).unwrap_or(1.0);
            max_zoom = Some(max_zoom.map_or(zoom, |m| m.max(zoom)));
            for u in uuids {
                if let Some(surf) = uuid_surface.get(u) {
                    best.entry(surf.id())
                        .and_modify(|e| if zoom > e.0 { *e = (zoom, surf.clone()); })
                        .or_insert_with(|| (zoom, surf.clone()));
                }
            }
        }
    }
    // Invisible-window strategy (`fractional_invisible` preference, live):
    // "off" — invisible windows keep receiving zoom-driven updates (the
    // historical behavior; backfilled below, since the render cull keeps them
    // out of the on-pane sets). "optimized" — invisible windows get no
    // publishes until visible again; parked worlds' windows get scale 1 so
    // their clients drop hi-res buffers. "full" — the hosted world's invisible
    // windows get scale 1 too. Capture targets are drawn, so they count as
    // visible in every mode.
    let mode = state.inner.preference.fractional_invisible.clone();
    let full = mode == "full";
    let optimized = full || mode == "optimized";
    if !optimized {
        // "off": feed the culled (but group-visible) windows back at the
        // sharpest zoom in play — historically they sat in every pane's drawn
        // set, so their best-zoom was the global max. Group-hidden windows
        // stay out, as they always were. No slots yet → nothing to emit.
        if let Some(max_zoom) = max_zoom {
            use compositor_y5_window_interface_draw::visible::DrawWindow;
            let unculled: Vec<WlSurface> = state
                .inner
                .space_state()
                .state
                .elements()
                .filter(|w| w.visible(state))
                .filter_map(|w| w.wl_surface().map(|s| s.into_owned()))
                .collect();
            for surf in unculled {
                best.entry(surf.id()).or_insert((max_zoom, surf));
            }
        }
    }
    let mut idle: Vec<WlSurface> = Vec::new();
    if optimized {
        // Windows of every non-hosted world: invisible by definition — scale 1
        // lets their clients drop hi-res buffers while the world is parked.
        for space in state.inner.other_world_spaces() {
            idle.extend(space.state.elements().filter_map(|w| w.wl_surface().map(|s| s.into_owned())));
        }
    }
    if full {
        // Hosted world, mapped but on no pane's screen (or group-hidden).
        // Capture targets are exempt: a recorded off-screen window keeps its
        // last real scale instead of degrading the recording to scale 1.
        let force = &state
            .inner
            .kernel
            .get(&compositor_orchestration_driver_capture_base::base::CAPTURE)
            .force_set;
        idle.extend(
            uuid_surface
                .iter()
                .filter(|(u, s)| !best.contains_key(&s.id()) && !force.contains(u))
                .map(|(_, s)| s.clone()),
        );
    }
    let per: Vec<(f64, WlSurface)> = best.into_values().collect();
    let emitted = FRAC_SENT.with(|sent| {
        compositor_support_smithay_state_fractional_dispatch::emit_best_per_surface(
            &mut state.state.fractional,
            &mut sent.borrow_mut(),
            &per,
            &idle,
        )
    });
    reassert_slots_after_scale(state, &emitted);
}

/// Re-state the compositor-decided size to every window that was just handed a NEW scale.
///
/// A scale change makes a client re-lay itself out, and a client that quantises its window to a
/// unit it cannot subdivide — a terminal's character cell — cannot land back on the size it was
/// configured to: it re-grids at the new cell size, keeps its cell COUNT, and commits whatever
/// logical size that rounds to. Nothing else re-configures it (the slot is unchanged; a zoom is
/// not a resize), so the divergence sticks, and the next scale change compounds it. Left alone
/// the window random-walks away from its slot and is cropped against it.
///
/// So arm the startup grace again here. It is the same jiggle used at map, aimed at the same
/// problem — a client that sizes itself against the compositor's decision — and it is
/// edge-triggered on the client's committed size, so a window that does NOT move costs nothing.
/// The client settles at or just under its slot, which the fit then renders 1:1
/// (see `fit::QUANTIZE_SLACK`).
///
/// Only `Decided` windows are re-asserted: an `Auto` window (a dialog / child toplevel) has been
/// left to size itself deliberately, and holding it to its own geometry would fight it.
fn reassert_slots_after_scale(state: &mut Loop, emitted: &[WlSurface]) {
    if emitted.is_empty() {
        return;
    }
    for window in state.inner.space_state().state.elements().cloned().collect::<Vec<_>>() {
        let Some(surface) = window.wl_surface() else { continue };
        if !emitted.iter().any(|s| s == surface.as_ref()) {
            continue;
        }
        let Some(decided) = compositor_y5_camera_transform_translate::slot::decided_size(&window)
        else {
            continue;
        };
        compositor_support_smithay_state_compositor_place::arm_size_propagation(&window, decided);
    }
}


/// What the renderer produced for `world` on `output`, or nothing.
///
/// Resolved at BIND time because it is per output — `collect` normalises every
/// rect to the pass's own extent, so the other monitor's set describes the same
/// window in the wrong UV space.
fn bind_frame_for(
    state: &Loop,
    world: Option<uuid::Uuid>,
    output: &std::sync::Arc<str>,
) -> compositor_pipeline_world_system_base::base::OutputFrame {
    let Some(w) = world else { return Default::default() };
    match state.inner.worlds.contains(w) {
        true => compositor_pipeline_world_system_base::base::frame(state.inner.worlds.get(w).storage(), output),
        false => Default::default(),
    }
}

/// Grace margin for the "full" fractional strategy, in world-logical units,
/// zoom-scaled the same way as the snap ranges (divided by zoom — so it is a
/// CONSTANT screen-space band around each pane, `range × output_scale` px,
/// regardless of zoom). Windows inside the band are published their real scale
/// before they scroll into view.
const FRACTIONAL_GRACE_RANGE: f64 = 256.0;

/// The "full" strategy's grace band: uuids of group-visible windows whose slot
/// rect, projected through the pane's camera (`render_target`), falls within
/// `pane` inflated by [`FRACTIONAL_GRACE_RANGE`]. These get their real
/// fractional scale ahead of reveal (instead of scale 1), so a pan-in never
/// shows a client still rendered at scale 1; they remain excluded from
/// rendering and frame callbacks.
fn grace_windows(
    state: &mut Loop,
    pane: smithay::utils::Rectangle<i32, Physical>,
) -> Vec<uuid::Uuid> {
    use compositor_y5_window_interface_draw::visible::DrawWindow;
    let ctx = state.viewport_context();
    let pad = (FRACTIONAL_GRACE_RANGE * ctx.scale).round() as i32;
    let windows: Vec<(uuid::Uuid, Window)> = state
        .inner
        .space_state()
        .state
        .elements()
        .filter(|w| w.visible(state))
        .filter_map(|w| w.uuid().map(|u| (u, w.clone())))
        .collect();
    let mut out = Vec::new();
    for (u, w) in windows {
        let Some(loc) = state.inner.space_state().state.element_location(&w) else {
            continue;
        };
        let sz = compositor_y5_camera_transform_translate::slot::expected_size(&w)
            .unwrap_or_else(|| w.geometry().size);
        if sz.w <= 0 || sz.h <= 0 {
            continue;
        }
        let project = |x: f64, y: f64| -> Point<i32, Physical> {
            let t: compositor_y5_camera_transform_translate::transform::Transform =
                ((x, y), ctx).into();
            t.into()
        };
        let tl = project(loc.x as f64, loc.y as f64);
        let br = project((loc.x + sz.w) as f64, (loc.y + sz.h) as f64);
        if tl.x < pane.loc.x + pane.size.w + pad
            && pane.loc.x - pad < br.x
            && tl.y < pane.loc.y + pane.size.h + pad
            && pane.loc.y - pad < br.y
        {
            out.push(u);
        }
    }
    out
}

/// Colour of the bar drawn between split viewport panes.
const SEPARATOR_COLOR: [f32; 4] = [0.16, 0.16, 0.19, 1.0];

/// Floating (detached) panes render as a contiguous stack ABOVE the tiled root's
/// content (`CANVAS` = 400): their background/backfill then their content, so a
/// detached pane sits entirely on top of everything in the root — root content
/// can't punch through between a floating pane's background and its windows.
const FLOATING_BG: layer::Layer = layer::Layer(401);
const FLOATING_CONTENT: layer::Layer = layer::Layer(402);
/// Border drawn around a detached (floating) viewport pane — its move/resize grab
/// zone — so the edge is visible.
const FLOATING_BORDER_COLOR: [f32; 4] = [0.30, 0.52, 0.92, 1.0];
const FLOATING_BORDER: i32 = 3;

/// Four edge bars framing `rect` (top, bottom, left, right), each `FLOATING_BORDER` thick.
fn border_edges(rect: smithay::utils::Rectangle<i32, Physical>) -> [smithay::utils::Rectangle<i32, Physical>; 4] {
    let (x, y, w, h, t) = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h, FLOATING_BORDER);
    [
        smithay::utils::Rectangle::from_loc_and_size((x, y), (w, t)),
        smithay::utils::Rectangle::from_loc_and_size((x, y + h - t), (w, t)),
        smithay::utils::Rectangle::from_loc_and_size((x, y), (t, h)),
        smithay::utils::Rectangle::from_loc_and_size((x + w - t, y), (t, h)),
    ]
}

thread_local! {
    /// Stable per-index element `Id`s for the separator bars, so smithay's
    /// damage tracking sees the same element across frames (a fresh `Id::new()`
    /// every frame would force a full repaint of each bar). Index = separator
    /// order from `viewport.layout`.
    static SEPARATOR_IDS: std::cell::RefCell<Vec<Id>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn separator_id(index: usize) -> Id {
    SEPARATOR_IDS.with(|cache| {
        let mut ids = cache.borrow_mut();
        while ids.len() <= index {
            ids.push(Id::new());
        }
        ids[index].clone()
    })
}

thread_local! {
    /// Stable per-pane element `Id`s for the per-region background clones (one
    /// parallax background is drawn per viewport pane). Distinct ids keep
    /// smithay's damage tracking from treating the panes' backgrounds as one.
    static BACKGROUND_IDS: std::cell::RefCell<Vec<Id>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn background_id(index: usize) -> Id {
    BACKGROUND_IDS.with(|cache| {
        let mut ids = cache.borrow_mut();
        while ids.len() <= index {
            ids.push(Id::new());
        }
        ids[index].clone()
    })
}

thread_local! {
    /// Stable per-edge element `Id`s for floating-pane border bars.
    static BORDER_IDS: std::cell::RefCell<Vec<Id>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Stable per-pane element `Id`s for floating panes' opaque black backfill.
    static FILL_IDS: std::cell::RefCell<Vec<Id>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn border_id(index: usize) -> Id {
    BORDER_IDS.with(|cache| {
        let mut ids = cache.borrow_mut();
        while ids.len() <= index {
            ids.push(Id::new());
        }
        ids[index].clone()
    })
}

fn fill_id(index: usize) -> Id {
    FILL_IDS.with(|cache| {
        let mut ids = cache.borrow_mut();
        while ids.len() <= index {
            ids.push(Id::new());
        }
        ids[index].clone()
    })
}

/// The GLES-built elements carried from the `prepare()` phase into the
/// renderer-agnostic `scene()`. iced UI, bevy 3D, and the parallax background
/// each render their content into GLES resources every frame (via the GLES
/// renderer), so they're produced here and handed to `scene()` as plain values
/// — `scene()` only wraps them into `SceneElement`s and draws them through the
/// `SceneDispatch` seam (real on GLES, blank on Vulkan).
pub struct PreparedGles {
    pub surfaces: Vec<compositor_monitor_compositor_iced_base::IcedRenderElement>,
    pub surfaces_screen: Vec<compositor_monitor_compositor_iced_base::IcedRenderElement>,
    /// Capture-dim layer elements, composited between windows and the
    /// world/background layers.
    pub surfaces_dim: Vec<compositor_monitor_compositor_iced_base::IcedRenderElement>,
    pub background_two: Option<compositor_background_two_draw_element::element::ParallaxBackground>,
    pub background_three: Vec<compositor_support_bevy_core_compositor_base::BevyRenderElement>,
    /// The embedded picker globe for the overview's World tab (empty otherwise).
    pub overview_world: compositor_y5_overview_draw_frame::frame::Prepared,
}

/// Per-pane cameras + sub-rects for the current output, matching how the content
/// band composites World iced (each leaf viewport through its own camera). This
/// recomputes the same `layout::compute` the content band uses; the duplication
/// is deliberate — backing (de)allocation needs the GLES renderer and must run in
/// `prepare()`, before the renderer-agnostic `scene()` builds the content band.
fn iced_panes(
    state: &Loop,
    size: Size<i32, Physical>,
) -> Vec<compositor_monitor_compositor_iced_base::PaneView> {
    let scale = state.viewport_context().scale;
    let vps = state.inner.viewports();
    let computed = compositor_y5_viewport_layout_base::layout::compute(
        vps,
        smithay::utils::Rectangle::from_loc_and_size(Point::from((0, 0)), size),
    );
    computed
        .regions
        .iter()
        .map(|r| {
            let cam = vps
                .camera_of(r.slot)
                .unwrap_or_else(|| vps.focus_camera())
                .transform
                .clone();
            compositor_monitor_compositor_iced_base::PaneView {
                transform: compositor_monitor_compositor_iced_base::Transform {
                    zoom: cam.zoom,
                    position: Point::new(cam.position.x * scale, cam.position.y * scale),
                },
                rect: r.rect,
            }
        })
        .collect()
}

/// GLES-only preparation phase: runs the per-frame hooks and builds the iced /
/// bevy / parallax GLES resources. Always runs on the (winit/native) GLES
/// renderer — separate from the renderer-agnostic `scene()` so the scene can be
/// composed by any renderer (e.g. Vulkan).
pub fn prepare(
    state: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> PreparedGles {
    // Temporary method of calling binding hooks for external renderers lazily.
    hooks::hooks(state, renderer, size);

    // De-alloc feature gate (`release_hidden_surfaces`). Set the flag up front so
    // `process_frame` (inside the surface scene below) sees this frame's value,
    // and when OFF skip ALL de-alloc compute — no panes, no draw-order, no
    // `manage_backings`, no occlusion — so the feature costs nothing while off.
    let dealloc = state.inner.preference.release_hidden_surfaces;
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.set_dealloc_enabled(dealloc);
    }

    let (surfaces, surfaces_screen, surfaces_dim) =
        compositor_y5_surface_draw_scene::scene::scene(state, renderer, size);

    // Iced backing lifecycle: release the dmabuf of any World surface not
    // actually visible in some pane (fully off-screen or fully obstructed), and
    // re-allocate on reveal. Uses the SAME per-pane cameras the content band
    // composites through — not render_all's single camera — so split / floating
    // panes resolve visibility correctly.
    if dealloc {
        let panes = iced_panes(state, size);
        let gpu = state.inner.environment.GPU.clone();
        // Real compositing z-order (topmost-first) so occlusion follows the
        // DrawOrder authority, not the registry's `items` Vec. World iced ids map
        // reversibly from their drawable uuid (see `surface.draw.handle::load`).
        let draw_order: Vec<compositor_monitor_compositor_iced_base::HandleId> = state
            .inner
            .drawable_order()
            .iter()
            .map(|u| compositor_monitor_compositor_iced_base::HandleId(u.as_u128() as u64))
            .collect();
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            reg.set_draw_order(&draw_order);
            reg.manage_backings(&gpu, renderer, size.to_f64(), &panes);
        }
    }
    // Parallax background is now a system (`TwoSystem`): it ticks its animation
    // in `update()` and emits a renderer-agnostic node from `draw()`. Run the
    // active world's draw pass, then bridge its `Background2D` node back into the
    // GLES prepare slot. The continuous-redraw cadence the parallax needs is a
    // driver concern, applied here while a node is live.
    let mut background_two = {
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
        frame.sorted().into_iter().find_map(|(_, node)| {
            node.downcast::<compositor_background_two_draw_element::element::ParallaxBackground>()
                .ok()
                .map(|b| *b)
        })
    };
    // An overlay world (e.g. lock) carries no parallax of its own, so the active
    // world's draw yields none — fall back to the focused session world's
    // (spawn_target). This keeps the real desktop background in the frame that the
    // lock capture blits, so the lock screenshot shows windows AND background.
    if background_two.is_none() {
        background_two = compositor_background_two_draw_scene::scene::scene(state);
    }
    // THE per-frame statement of what the background on screen needs from the
    // engine: collect the world set, leave the band to the shader, redraw whole.
    //
    // Here, and not at selection time, because selection happens once per BUNDLE
    // while these are properties of the WORLD BEING DRAWN. `load_multipass` runs
    // only when a world builds its `ParallaxBackground`, so a world returning to
    // the screen with an already-compiled bundle never restated them and the
    // engine went on following whichever world built last — open the picker, come
    // back, and the session world is drawn under the picker's answers.
    //
    // And here specifically, after BOTH branches above, because this is where the
    // two ways a background can arrive converge: the active world's own draw, and
    // the `spawn_target` fallback an overlay world takes. Publishing inside the
    // fallback alone reaches only the overlay case — which blanks the screen for
    // the ordinary one, since a band-owning bundle then draws with an empty world
    // set while the engine has been told to leave the band alone.
    if let Some(w) = background_two.as_ref().and_then(|b| b.world) {
        if state.inner.worlds.contains(w) {
            compositor_pipeline_world_system_base::base::publish_facts(
                state.inner.worlds.get_mut(w).storage_mut(),
                background_two.as_ref().and_then(|b| b.pipeline.as_deref()),
            );
        }
    }
    if background_two.is_some() {
        state.schedule_redraw_post_vblank();
    }
    let background_three =
        compositor_background_three_draw_scene::scene::scene(state, renderer, size);

    // (Lock engage is no longer a per-frame drain — the keybinding sets the lock
    // status synchronously and runs `lock_logical` off-frame; the lock VISUAL is
    // built lazily in `lock.scene/scene.frame::prepare`.)

    // World-selection screen: a two-frame capture-on-leave. Frame A (the request
    // is present) arms a framebuffer capture of the still-active origin world;
    // this frame's render fills it. Frame B (request cleared, capture armed)
    // snapshots it into the origin's thumbnail and switches to the picker. Both
    // run here, while the origin world is the one being drawn.
    if state.inner.__set_picker.take().is_some() {
        compositor_y5_picker_interface_capture::capture::arm(state, renderer, size);
    } else {
        compositor_y5_picker_interface_capture::capture::finish_arm_and_open(state, renderer, size);
    }

    // Overview (Super+Tab): the freeze-backdrop capture + the World-tab globe are
    // owned by the overview layer; this is its GLES-phase hook.
    let overview_world = compositor_y5_overview_draw_frame::frame::prepare(state, renderer, size);

    PreparedGles {
        surfaces,
        surfaces_screen,
        surfaces_dim,
        background_two,
        background_three,
        overview_world,
    }
}

pub fn scene<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    prepared: PreparedGles,
) -> Scene<R>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    // Handles incoming buffers such as the RPC buffer.
    buffers::update(state, renderer, size);

    // Invoke state machine updates, such as the navigator state machine

    // Assemble the frame as a layered plan of owned draw nodes, then lower it
    // to the renderer's element list at the single backend seam. Layering is
    // explicit (BACKGROUND..POINTER); contributors no longer rely on push order.
    let mut plan: Plan<R> = Plan::new();
    // This output's retrace count + refresh, for the off-thread background's
    // per-pane pacing. Bumped once per output per scene build, which on the
    // native path is once per vblank.
    let frame_serial = state.inner.next_frame_serial();
    let refresh = state.inner.current_refresh();
    // Publish the FASTEST panel for the off-thread UI worker, which has no output
    // of its own to ask. Deliberately not `refresh` (this output's): `scene` runs
    // once per output, so publishing the drawing output's value made the global
    // whatever monitor drew last, alternating every frame on a mixed-refresh
    // desktop. Fastest rather than slowest because bevy elements are not gated
    // per output — see `Orchestrator::fastest_refresh`.
    compositor_model_environment_interface_base::base::set_refresh(
        state.inner.fastest_refresh(),
    );

    // Screen-space overlays (the cursor, layer-shell, and screen iced like the
    // launcher / settings window) belong to ONE output — the one under the cursor
    // (fallback: the primary). Because the kernel now calls scene() once PER
    // physical output, gate them so they aren't duplicated + mispositioned on every
    // monitor. `render_output == None` = a non-loop pass (winit / single) → draw all.
    let surfaces_screen = prepared.surfaces_screen;
    let render_key = state.inner.render_output.clone();
    // Hoisted: the background pane key is derived from it once per output pass,
    // not once per pane per frame.
    // An `Arc<str>`, not a `String`: every region of this pass binds a pane key
    // from it, so they share one allocation instead of copying the key each. This
    // line already cloned a `String`, so the cost is unchanged.
    let pane_output: std::sync::Arc<str> =
        std::sync::Arc::from(render_key.clone().unwrap_or_default().as_str());
    let draw_screen = match &render_key {
        None => true,
        Some(key) => {
            let active = state.inner.cursor_output.clone().or_else(|| {
                state
                    .inner
                    .space_state()
                    .state
                    .outputs()
                    .next()
                    .map(compositor_orchestration_core_state_base::state::output_key)
            });
            active.as_deref() == Some(key.as_str())
        }
    };
    // Per-element gate for screen-space iced. An UNBOUND surface (launcher,
    // dialogs, cursor) belongs to the one active output → `draw_screen`. An
    // OUTPUT-BOUND surface (per-monitor capture overlays) draws only on ITS
    // output, regardless of which monitor the cursor is on. `render_key == None`
    // (winit / single-output pass) draws everything.
    let draw_iced = |output: &Option<String>| match output {
        None => draw_screen,
        Some(tag) => render_key.as_deref().map_or(true, |k| k == tag.as_str()),
    };
    if draw_screen {
        // Picker entry, FIRST half: the world being left ramps to black before the
        // switch (the picker's scene clears the same overlay on the far side).
        // Pushed ahead of the pointer, so — like the picker's own — it covers the
        // cursor too; nothing should survive the fade.
        if let Some(solid) = compositor_y5_picker_scene_fade::fade::leaving(state, size) {
            plan.push(layer::POINTER, DrawNode::Solid(solid));
        }
        let pointer = compositor_orchestration_seat_pointer_draw::scene::element(state, renderer, size);
        plan.extend(layer::POINTER, pointer.into_iter().map(DrawNode::Pointer));
    }

    // Split the four wlr layer-shell layers into their own render bands so z-order
    // matches hit-testing: Background/Bottom under content, Top/Overlay above windows
    // but below the compositor's screen iced.
    //
    // NOT gated by `draw_screen`: layer surfaces are OUTPUT-BOUND, so each output draws
    // its OWN bars (via `render_key`) regardless of which monitor the cursor is on —
    // unlike the unbound cursor/launcher, which follow the active output. A bar therefore
    // stays put on its monitor instead of chasing the cursor / duplicating onto others.
    //
    // While the overview overlay is open, suppress the LIVE layer bands: the frozen
    // freeze-backdrop already captured them (like windows), so rendering them live on top
    // would leave them animating above the frozen desktop. Windows get the same treatment
    // (the overview owns the content band).
    if !state.inner.overview().visible {
        for (wlr_layer, node) in layershell(state, size, render_key.as_deref()) {
            let band = match wlr_layer {
                WlrLayer::Background => layer::LAYER_BACKGROUND,
                WlrLayer::Bottom => layer::LAYER_BOTTOM,
                WlrLayer::Top => layer::LAYER_TOP,
                WlrLayer::Overlay => layer::LAYER_OVERLAY,
            };
            plan.push(band, DrawNode::Surface(node));
        }
    }
    for elem in surfaces_screen {
        if draw_iced(&elem.output) {
            plan.push(layer::ICED_SCREEN, DrawNode::Iced(elem));
        }
    }


    // CONTENT band. The overview overlay (Super+Tab) owns this band when open
    // (backdrop + grid/globe) — its layer handles it and returns the windows it
    // drew; otherwise draw the normal canvas: windows + world iced INTERLEAVED by
    // the DrawOrder authority ("everything interleaves").
    let canvas_window = match compositor_y5_overview_draw_frame::frame::band(
        state,
        renderer,
        size,
        &mut plan,
        prepared.overview_world,
    ) {
        Some(windows) => {
            // Overview overlay owns the content band; keep the background full-screen.
            if let Some(mut bg) = prepared.background_two.clone() {
                // This branch builds its own plan and so never reaches the region
                // loop's `bind_pane` below — it has to supply the pane identity
                // itself, exactly as the picker and the lock screen do.
                //
                // Without it the element kept the constructor's placeholders:
                // pane 0, the 30Hz fallback refresh, and a `serial` frozen at 0.
                // Frozen is the damaging one — under `Cadence::Vblank` the rate
                // gate asks `serial - last_serial >= need`, which is false forever
                // once the serial stops advancing, so the only thing still
                // admitting a render is the stall rescue at `min_interval * 3`.
                // That is a third of the configured rate, whatever the setting says.
                //
                // Its OWN namespace, not the world's region 0: those are the same
                // rect only while the viewport is unsplit. Sharing the key would
                // have `ensure` reallocate the ring to the full output on every
                // overview open and back to the sub-rect on every close.
                bg.bind_frame(bind_frame_for(state, bg.world, &pane_output));
                bg.bind_overlay("overview", &pane_output, refresh, frame_serial);
                plan.push(layer::BACKGROUND, DrawNode::Background2D(bg));
            }
            windows
        }
        None => {
            // Region pass: one world, drawn once per leaf viewport (split / floating
            // pane) through that slot's camera into its sub-rect. `render_target`
            // makes the focus accessors (camera/size_context) resolve to the pane
            // being drawn; the canvas/window scene then projects + crops into it.
            // A single default slot yields one full-output region (unchanged path).
            let scale = state.size_ctx_all().scale;
            let computed = compositor_y5_viewport_layout_base::layout::compute(
                state.inner.viewports(),
                smithay::utils::Rectangle::new(Point::from((0, 0)), size),
            );
            let mut cw = Vec::new();
            state.inner.viewports_mut().on_pane_grace.clear();
            state.inner.viewports_mut().on_pane_awake.clear();
            // Back-to-front: regions are root-first then floating; within a layer
            // the first-pushed element is front-most, so iterate in reverse to draw
            // floating panes (and their backgrounds) on top of the tiled root.
            for (region_index, region) in computed.regions.iter().enumerate().rev() {
                state.inner.render_target = Some(
                    compositor_orchestration_core_state_base::state::RenderTarget {
                        slot: region.slot,
                        origin_logical: (region.rect.loc.x as f64 / scale, region.rect.loc.y as f64 / scale),
                        size_physical: (region.rect.size.w as f64, region.rect.size.h as f64),
                    },
                );
                // Floating (detached) panes stack ABOVE the root content; tiled
                // root panes use the base bands. Both get the per-pane parallax;
                // floating panes ALSO get an opaque black backfill behind the
                // parallax (its shader clear leaves them partly transparent).
                let is_floating = state.inner.viewports().root.find(region.slot).is_none();
                let (bg_layer, content_layer) = if is_floating {
                    (FLOATING_BG, FLOATING_CONTENT)
                } else {
                    (layer::BACKGROUND, layer::CANVAS)
                };
                if let Some(base) = prepared.background_two.as_ref() {
                    let (pan_x, pan_y, zoom) = {
                        let t = &state.inner.camera().transform;
                        (t.position.x as f32, t.position.y as f32, t.zoom as f32)
                    };
                    let mut bg = base.clone();
                    bg.bind_frame(bind_frame_for(state, bg.world, &pane_output));
                    bg.bind_pane(
                        (region.rect.loc.x, region.rect.loc.y),
                        (region.rect.size.w as f32, region.rect.size.h as f32),
                        (pan_x, pan_y),
                        zoom,
                        background_id(region_index),
                        // OUTPUT + region, never the region index alone. Element
                        // ids may repeat across monitors because smithay's damage
                        // tracker is per-output, but the worker's pane map is
                        // process-global — and regions are numbered per output,
                        // restarting at 0 on each. Keyed on the index alone, every
                        // monitor's root pane collided on 0: one buffer, one
                        // camera, one size for all of them, and on mixed
                        // resolutions `ensure` reallocated the ring twice a frame
                        // as the two sizes fought over it.
                        //
                        // The third component, the world, is NOT supplied here: it
                        // is stamped on the element where it was built, because an
                        // overlay draws the spawn target's background rather than
                        // the active world's. `bind_pane` assembles the key.
                        &pane_output,
                        region_index,
                        // This monitor's refresh + retrace count: the worker paces
                        // each pane against the display it is actually shown on.
                        refresh,
                        frame_serial,
                        // Region count, so a viewport collapse retires the panes it
                        // left behind at once instead of on a liveness timeout.
                        computed.regions.len(),
                    );
                    if is_floating {
                        // Hard-clip to the pane rect: the parallax shader's clear
                        // follows the damage and would otherwise leak beyond a
                        // floating pane. Full-output root panes need no clip.
                        // The crop is applied at lower() time, so the off-thread
                        // path can substitute its texture first.
                        plan.push(
                            bg_layer,
                            DrawNode::Background2DCropped { elem: bg, crop: region.rect },
                        );
                    } else {
                        plan.push(bg_layer, DrawNode::Background2D(bg));
                    }
                }
                if is_floating {
                    // Behind the parallax (pushed after → lower in-band), in front
                    // of the root: guarantees the detached pane is fully opaque.
                    let fill = SolidColorRenderElement::new(
                        fill_id(region_index),
                        region.rect,
                        CommitCounter::default(),
                        [0.0, 0.0, 0.0, 1.0],
                        Kind::Unspecified,
                    );
                    plan.push(bg_layer, DrawNode::Solid(fill));
                }
                let (content, vis) =
                    compositor_y5_canvas_draw_scene::scene::scene(state, renderer, size);
                for item in content {
                    // Per-pane clipping is applied inside the window/decoration/
                    // cursor builders (they read `render_target` and crop to the
                    // pane rect), where the element geometry is well-defined.
                    match item {
                        compositor_y5_canvas_draw_scene::scene::ContentItem::Canvas { elem, flags, times } => {
                            plan.push(content_layer, DrawNode::Canvas { elem, flags, times })
                        }
                        compositor_y5_canvas_draw_scene::scene::ContentItem::Iced(e) => {
                            // World-space iced surfaces clip to the pane (native +
                            // GLES paths handled in `DrawNode::lower`).
                            plan.push(content_layer, DrawNode::IcedCropped { elem: e, crop: region.rect })
                        }
                    }
                }
                // The window scene culls twice (capture targets exempt from both),
                // and the two sets it returns feed different consumers:
                //
                // `vis.drawn` — passed the frustum AND was not fully covered by
                // opaque windows in front of it. This is the presented set
                // (`visible_window` → frame callbacks + presentation feedback):
                // a window that contributed no pixels is not owed either.
                //
                // `vis.on_pane` — passed the frustum, covered or not. This is the
                // per-slot fractional-scale set. Occlusion deliberately does NOT
                // narrow it: an occluded window is revealed the instant the
                // window over it moves or closes, with no pan to hide a scale
                // republish behind, so it must keep its real scale.
                //
                // A window panned off every pane (and not captured) is in neither
                // — it stops re-rendering and receiving scale updates until
                // revealed.
                let mut uuids: Vec<uuid::Uuid> = vis.on_pane.iter().filter_map(|w| w.uuid()).collect();
                // "full" grace band: windows just outside the pane get their REAL
                // scale published ahead of reveal, so panning them in doesn't flash
                // a stale scale-1 buffer. Fractional-only — they are still culled
                // from rendering and frame callbacks.
                if state.inner.preference.fractional_invisible == "full" {
                    let present: std::collections::HashSet<uuid::Uuid> =
                        uuids.iter().copied().collect();
                    uuids.extend(
                        grace_windows(state, region.rect)
                            .into_iter()
                            .filter(|u| !present.contains(u)),
                    );
                }
                // The `suspended` set for this pane: on the pane, or off it inside the
                // SUSPEND band. Sibling of `on_pane_grace` above, with its own wider and
                // unconditional band — see `Viewports::on_pane_awake`. Computed by the
                // cull, which had both rects, not by re-projecting every window here.
                let on_pane_awake: Vec<uuid::Uuid> =
                    vis.on_pane_awake.iter().filter_map(|w| w.uuid()).collect();
                state.inner.viewports_mut().on_pane_awake.insert(region.slot, on_pane_awake);
                state.inner.viewports_mut().on_pane_grace.insert(region.slot, uuids);
                cw.extend(vis.drawn);
            }
            state.inner.render_target = None;
            // Wide bars between split panes (drawn above window content).
            for (i, sep) in computed.separators.iter().enumerate() {
                let bar = SolidColorRenderElement::new(
                    separator_id(i),
                    sep.rect,
                    CommitCounter::default(),
                    SEPARATOR_COLOR,
                    Kind::Unspecified,
                );
                plan.push(layer::ICED_SCREEN, DrawNode::Solid(bar));
            }
            // Borders around detached (floating) panes — the visible move/resize
            // grab frame. Drawn above content; one id per edge for damage tracking.
            for (fi, v) in state.inner.viewports().floating.iter().enumerate() {
                if let compositor_y5_viewport_state_base::state::Viewport::Floating { rect, .. } = v {
                    for (ei, edge) in border_edges(*rect).into_iter().enumerate() {
                        let bar = SolidColorRenderElement::new(
                            border_id(fi * 4 + ei),
                            edge,
                            CommitCounter::default(),
                            FLOATING_BORDER_COLOR,
                            Kind::Unspecified,
                        );
                        plan.push(layer::ICED_SCREEN, DrawNode::Solid(bar));
                    }
                }
            }
            cw
        }
    };

    // Capture-dim backdrop: below the content band, above background. Bound to
    // the capture's origin output, so it dims only that monitor (correctly
    // sized) instead of every monitor at the origin's dimensions.
    for elem in prepared.surfaces_dim {
        if draw_iced(&elem.output) {
            plan.push(layer::CAPTURE_DIM, DrawNode::Iced(elem));
        }
    }
    let _ = prepared.surfaces; // world iced now drawn per-id in the content band
    plan.extend(layer::WORLD_3D, prepared.background_three.into_iter().map(DrawNode::Background3D));
    // The parallax background is pushed per-pane in the content match above (one
    // per viewport pane, or full-screen for the overview overlay).

    let (elements, meta) = plan.lower(renderer);

    // Per-window fractional scale: each window follows its HIGHEST-zoom viewport
    // across ALL outputs (best resolution wins), emitted only when a surface's scale
    // changes. Computed from live per-output view state (not per-output during the
    // pass), so a window on two differently-zoomed monitors doesn't get its scale
    // flip-flopped — and re-sent to the client — every frame.
    update_fractional(state);

    // Auto-tick the resize debounce per frame: emit any due (throttled) `send_configure` even
    // without a pointer motion, so a mid-drag pause re-renders the client to the paused size on
    // its own instead of waiting for the next motion / release. Collect first (borrows the space),
    // then send.
    let resize_due: Vec<(Window, Size<i32, Logical>)> = state
        .inner.space_state()
        .state
        .elements()
        .filter_map(|w| {
            compositor_y5_camera_transform_translate::slot::resize_due(w).map(|s| (w.clone(), s))
        })
        .collect();
    for (window, size) in resize_due {
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|s| s.size = Some(size));
            toplevel.send_configure();
        }
    }

    // Return the resulting scene
    Scene {
        Element: elements,
        meta,
        visible_window: canvas_window,
    }
}

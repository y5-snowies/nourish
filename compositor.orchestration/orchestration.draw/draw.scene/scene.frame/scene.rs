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
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use compositor_orchestration_draw_scene_element::element::SceneElement;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_draw_node_base::node::{DrawNode, Plan};
use compositor_support_system_world_frame_base::base as layer;

pub struct Scene<R: Renderer> {
    pub Element: Vec<SceneElement<R>>,
    pub visible_window: Vec<Window>,
}

thread_local! {
    /// Last fractional scale emitted per surface — the dedup so `update_fractional`
    /// only re-sends `wp_fractional_scale` when a window's best-resolution scale
    /// actually changes, not every frame. Keyed by the surface's protocol id.
    static FRAC_SENT: std::cell::RefCell<
        std::collections::HashMap<smithay::reexports::wayland_server::backend::ObjectId, f64>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Per-window fractional scale, best-resolution across ALL outputs. A window may be
/// visible in several viewports spread over several monitors at different zooms; its
/// preferred scale follows the HIGHEST-zoom (sharpest) one. Derived from the live
/// per-output view state (`output_views`: each slot's camera zoom + its `visible`
/// window set), so it's independent of which output is mid-render — and emitted only
/// on change (via `FRAC_SENT`), which is what stops the per-output flip-flop from
/// re-sending the scale to clients every frame.
fn update_fractional(state: &mut Loop) {
    use smithay::reexports::wayland_server::Resource;
    // uuid → surface for currently-mapped windows (the `visible` sets store uuids).
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
    for vps in state.inner.output_views().map.values() {
        for (slot, uuids) in &vps.visible {
            let zoom = vps.camera_of(*slot).map(|c| c.transform.zoom).unwrap_or(1.0);
            for u in uuids {
                if let Some(surf) = uuid_surface.get(u) {
                    best.entry(surf.id())
                        .and_modify(|e| if zoom > e.0 { *e = (zoom, surf.clone()); })
                        .or_insert_with(|| (zoom, surf.clone()));
                }
            }
        }
    }
    let per: Vec<(f64, WlSurface)> = best.into_values().collect();
    FRAC_SENT.with(|sent| {
        compositor_support_smithay_state_fractional_dispatch::emit_best_per_surface(
            &state.state.fractional,
            &mut sent.borrow_mut(),
            &per,
        );
    });
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
    pub overview_world: Vec<compositor_support_bevy_core_compositor_base::BevyRenderElement>,
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

    let (surfaces, surfaces_screen, surfaces_dim) =
        compositor_y5_surface_draw_scene::scene::scene(state, renderer, size);
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

    // Screen-space overlays (the cursor, layer-shell, and screen iced like the
    // launcher / settings window) belong to ONE output — the one under the cursor
    // (fallback: the primary). Because the kernel now calls scene() once PER
    // physical output, gate them so they aren't duplicated + mispositioned on every
    // monitor. `render_output == None` = a non-loop pass (winit / single) → draw all.
    let surfaces_screen = prepared.surfaces_screen;
    let render_key = state.inner.render_output.clone();
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
        let pointer = compositor_orchestration_seat_pointer_draw::scene::element(state, renderer, size);
        plan.extend(layer::POINTER, pointer.into_iter().map(DrawNode::Pointer));

        let layer_shell = layershell(state, size);
        plan.extend(layer::LAYERSHELL, layer_shell.into_iter().map(DrawNode::Surface));
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
            if let Some(bg) = prepared.background_two.clone() {
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
            state.inner.viewports_mut().visible.clear();
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
                    bg.bind_pane(
                        (region.rect.loc.x, region.rect.loc.y),
                        (region.rect.size.w as f32, region.rect.size.h as f32),
                        (pan_x, pan_y),
                        zoom,
                        background_id(region_index),
                    );
                    if is_floating {
                        // Hard-clip to the pane rect: the parallax shader's clear
                        // follows the damage and would otherwise leak beyond a
                        // floating pane. Full-output root panes need no clip.
                        if let Some(cropped) = smithay::backend::renderer::element::utils::CropRenderElement::from_element(
                            bg,
                            Scale::from(1.0),
                            region.rect,
                        ) {
                            plan.push(bg_layer, DrawNode::Background2DCropped(cropped));
                        }
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
                        compositor_y5_canvas_draw_scene::scene::ContentItem::Canvas(e) => {
                            plan.push(content_layer, DrawNode::Canvas(e))
                        }
                        compositor_y5_canvas_draw_scene::scene::ContentItem::Iced(e) => {
                            // World-space iced surfaces clip to the pane (native +
                            // GLES paths handled in `DrawNode::lower`).
                            plan.push(content_layer, DrawNode::IcedCropped { elem: e, crop: region.rect })
                        }
                    }
                }
                // Record which windows are visible in this pane, for the per-window
                // fractional scale (computed cross-output after the pass — see
                // `update_fractional`, which reads each slot's camera zoom + visible).
                let uuids: Vec<uuid::Uuid> = vis.iter().filter_map(|w| w.uuid()).collect();
                state.inner.viewports_mut().visible.insert(region.slot, uuids);
                cw.extend(vis);
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

    let elements = plan.lower(renderer);

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
        visible_window: canvas_window,
    }
}

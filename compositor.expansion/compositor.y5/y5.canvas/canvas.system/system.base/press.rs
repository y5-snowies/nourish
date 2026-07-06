//! Pointer PRESS, migrated from the rim (`canvas.input/input.pointer/press.rs`
//! + `window.input/input.pointer/window.rs`) into `CanvasSystem::input`.
//!
//! Consume is ALL-OR-NOTHING: this returns `InputFlow::Consume` exactly when the
//! old rim handler returned `true` (`!_temporary_passthrough`), and
//! `InputFlow::Pass` when it returned `false` (so the rim's `native_press` runs
//! against the window beneath). The grab is written via CanvasSystem's own
//! buffer; selection via the SELECT_REQUEST channel; iced focus/button via the
//! surface system channels; the wayland focus/button + held-key release via
//! `cx.seat`; window-deactivate + grab geometry via `cx.platform.space()`.

use compositor_support_system_input_event_base::base::InputFlow;
use compositor_support_system_storage_slot_base::base::Storage;
use compositor_support_system_trait_system_base::base::SystemCx;
use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_y5_canvas_input_state::state::{
    ActiveOption, ActiveTransformCandidate, Anchor, CanvasGrab, SnapMap, SnapSource, TargetOption,
};
use compositor_y5_viewport_state_base::state::OUTPUT_VIEWS;
use compositor_y5_group_state_base::state::{Group, GroupVisibility, GROUP};
use compositor_y5_placeholder_system_base::base::PLACEHOLDER;
use compositor_y5_select_state_base::request::{announce_selection, SelectionCmd};
use compositor_y5_select_state_base::select::SELECT;
use compositor_y5_surface_interface_core::hit::surface_under_filtered_cx;
use compositor_y5_surface_system_base::base::{announce_iced_button, announce_iced_focus};
use compositor_y5_window_interface_record::window::LoopWindow;
use compositor_monitor_compositor_iced_base::HandleId;
use smithay::backend::input::{ButtonState, KeyState};
use smithay::desktop::Window;
use smithay::input::keyboard::Keycode;
use smithay::input::pointer::ButtonEvent;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size, SERIAL_COUNTER};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use crate::base::{CanvasCmd, CANVAS, CANVAS_BUF};
use crate::snap;

/// A pressable content target (mirrors the rim's `PressCandidate`).
enum PressCandidate {
    IcedSurface(HandleId),
    Window(Window),
}

pub(crate) fn press(cx: &mut SystemCx, button: u32, x: f64, y: f64) -> InputFlow {
    let cursor = Point::<f64, Logical>::from((x, y));

    let mut canvas_grab_targetting = false;
    let mut canvas_grab_selecting = false;
    let mut canvas_grab_hand = false;
    match &cx.storage.get(&CANVAS).Grab {
        CanvasGrab::Target(opt) => match opt {
            TargetOption::Scale | TargetOption::Move => canvas_grab_targetting = true,
            TargetOption::Select { .. } => {
                canvas_grab_targetting = true;
                canvas_grab_selecting = true;
            }
        },
        CanvasGrab::Active(ActiveOption::Hand) => canvas_grab_hand = true,
        _ => {}
    }

    // Hit-test (press accepts a window only if visible; layers/iced pass). While
    // the overview overlay is open the view is presentational — reject every
    // window hit so clicks never reach a window; iced (the menu bar) still routes.
    let overview_open =
        cx.storage.get(&compositor_y5_overview_state_base::base::OVERVIEW).visible;
    let over_surface = surface_under_filtered_cx(cx.storage, cursor, &|hit| {
        if let Some(window) = hit.window() {
            return !overview_open && window_visible(cx.storage, window);
        }
        true
    });
    // "Over ice" = over any iced surface (registry `Iced` hits, or a layer-shell
    // surface flagged ice). Registry `Iced` hits must count here too, otherwise a
    // click on a world-space iced UI (e.g. the selection toolbar) falls through to
    // the canvas press and clears the selection — destroying a selection-driven
    // overlay before its button can act.
    let over_ice =
        matches!(&over_surface, Some(hit) if hit.ice().is_some() || hit.is_iced());

    let mut temporary_passthrough = false;
    if let Some(ice_layer) = over_surface.as_ref().and_then(|w| w.iced_layer()) {
        temporary_passthrough = (ice_layer
            & compositor_orchestration_draw_layer_base::base::Layer::SCENE_SURFACE_GROUP.bits())
            != 0;
    }

    // Over a window and not targeting: clear selection (unless over ice) and Pass
    // so the rim's native_press routes the click to the window.
    if !canvas_grab_hand
        && !canvas_grab_targetting
        && (over_surface.is_some() && !temporary_passthrough)
    {
        if !over_ice {
            let cleared = cx.storage.get(&SELECT).clear();
            announce_selection(cx.channels, SelectionCmd::Set(cleared));
        }
        return InputFlow::Pass;
    }

    if !canvas_grab_hand
        && canvas_grab_targetting
        && let Some(hit) = &over_surface
    {
        let candidate: Option<PressCandidate> = if let Some(w) = hit.window() {
            Some(PressCandidate::Window(w.clone()))
        } else if let Some(h) = hit.iced_handle() {
            Some(PressCandidate::IcedSurface(h))
        } else {
            None
        };

        if candidate.is_none() && over_ice {
            // Let ice dispatch (transitive with canvas ownership).
            return InputFlow::Pass;
        }

        if let Some(candidate) = candidate {
            // Grabbing a window directly (Move/Scale) brings it to the front and
            // focuses it, like a plain click — the targeting branch bypasses the
            // rim's native_press (which used to do this). Select is exempt: it only
            // mutates the selection set, not focus/stacking.
            if let PressCandidate::Window(window) = &candidate {
                if !canvas_grab_selecting {
                    raise_focus_window(cx, window);
                }
            }
            trigger_grab(cx, &candidate, cursor);
        }
    } else {
        // Not targeting, or the hit was not over a window.
        if !canvas_grab_hand && over_ice {
            return InputFlow::Pass;
        }

        // If not targeting anything, clear the selection.
        if !canvas_grab_hand && !canvas_grab_selecting {
            let cleared = cx.storage.get(&SELECT).clear();
            announce_selection(cx.channels, SelectionCmd::Set(cleared));
        }

        // Shift held (Select append): start a select box; otherwise start a pan.
        let select_append =
            matches!(&cx.storage.get(&CANVAS).Grab, CanvasGrab::Target(TargetOption::Select { Append }) if *Append);
        if select_append {
            let start_selection: Vec<Uuid> = cx
                .storage
                .get(&SELECT)
                .Selection
                .iter()
                .filter_map(|w| w.uuid())
                .collect();
            cx.write(
                &CANVAS_BUF,
                CanvasCmd::SetGrab(CanvasGrab::Active(ActiveOption::SelectBox {
                    start_cursor: cursor,
                    current_cursor: cursor,
                    start_selection,
                })),
            );
        } else {
            cx.write(&CANVAS_BUF, CanvasCmd::PanUpdating(true));
        }

        if !canvas_grab_hand {
            finalize_non_hand(cx, button);
        }
    }

    if temporary_passthrough {
        InputFlow::Pass
    } else {
        InputFlow::Consume
    }
}

/// Deactivate every window, release held keys, drop wayland + iced keyboard
/// focus, and forward the press to the seat/iced (the rim's non-hand tail).
fn finalize_non_hand(cx: &mut SystemCx, button: u32) {
    let serial = SERIAL_COUNTER.next_serial();
    let time = now_msec();

    if let Some(platform) = cx
        .platform
        .as_deref_mut()
        .and_then(|p| p.downcast_mut::<compositor_orchestration_draw_platform_base::platform::Platform>())
    {
        for window in platform.space().elements() {
            window.set_activated(false);
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
    }

    if let Some(dispatch) = cx.seat.as_deref_mut().and_then(|s| s.downcast_mut::<Dispatch>()) {
        // Release held (non-modifier) keys before dropping focus so clients that
        // track their own keyboard state don't get stuck key-down on re-focus.
        release_held_keys(dispatch);

        if let Some(keyboard) = dispatch.seat.seat.get_keyboard() {
            keyboard.set_focus(dispatch, Option::<WlSurface>::None, serial);
        }

        if let Some(pointer) = dispatch.seat.seat.get_pointer() {
            pointer.button(
                dispatch,
                &ButtonEvent { button, state: ButtonState::Pressed, serial, time },
            );
            pointer.frame(dispatch);
        }
    }

    // Iced deactivation: clear keyboard focus + dispatch the button-down. Both
    // route through the surface system's slot (we can't touch its registry).
    announce_iced_focus(cx.channels, None);
    announce_iced_button(cx.channels, button, true);
}

/// Bring a directly-grabbed window to the front and give it keyboard focus —
/// the equivalent of the click path (`native_press/press.rs:51-61`) which the
/// canvas targeting branch bypasses. Three parts must stay in sync with that path:
///   1. smithay space order (`raise_element`) — keyboard/hit fallback ordering,
///   2. the world DRAW_ORDER authority (the actual rendered top-level z) — routed
///      through the buffer (`CanvasCmd::RaiseDrawable`) since `Platform` only
///      exposes the `Space`, not DRAW_ORDER,
///   3. per-window activation + keyboard focus on the toplevel surface.
fn raise_focus_window(cx: &mut SystemCx, window: &Window) {
    let Some(uuid) = window.uuid() else { return };

    // Smithay space order + per-window activation.
    if let Some(platform) = cx
        .platform
        .as_deref_mut()
        .and_then(|p| p.downcast_mut::<compositor_orchestration_draw_platform_base::platform::Platform>())
    {
        platform.space().raise_element(window, true);
        for w in platform.space().elements() {
            w.set_activated(w == window);
            if let Some(toplevel) = w.toplevel() {
                toplevel.send_pending_configure();
            }
        }
    }

    // Draw-order authority — the actual visual top-level (buffer phase).
    cx.write(&CANVAS_BUF, CanvasCmd::RaiseDrawable(uuid));

    // Keyboard focus on the window's toplevel surface.
    if let Some(surface) = window.toplevel().map(|t| t.wl_surface().clone()) {
        if let Some(dispatch) = cx.seat.as_deref_mut().and_then(|s| s.downcast_mut::<Dispatch>()) {
            if let Some(keyboard) = dispatch.seat.seat.get_keyboard() {
                let serial = SERIAL_COUNTER.next_serial();
                keyboard.set_focus(dispatch, Some(surface), serial);
            }
        }
    }
}

/// Reimplementation of `compositor_orchestration_seat_keyboard_input::keyboard::
/// release_held_keys` reading `cx.seat`'s `Dispatch` instead of `&mut Loop`:
/// release every held NON-modifier key to the focused client (forwarded, no
/// intercept) before keyboard focus is cleared.
fn release_held_keys(dispatch: &mut Dispatch) {
    let Some(keyboard) = dispatch.seat.seat.get_keyboard() else {
        return;
    };
    let to_release: Vec<Keycode> = keyboard.with_pressed_keysyms(|syms| {
        syms.iter()
            .filter(|h| !is_modifier_keysym(h.modified_sym().raw()))
            .map(|h| h.raw_code())
            .collect()
    });
    if to_release.is_empty() {
        return;
    }
    let time = now_msec();
    for key in to_release {
        let serial = SERIAL_COUNTER.next_serial();
        let _ = keyboard.input::<(), _>(dispatch, key, KeyState::Released, serial, time, |_, _, _| {
            smithay::input::keyboard::FilterResult::Forward
        });
    }
}

/// X11 keysym ranges for modifier keys (see the rim `keyboard.rs` copy).
fn is_modifier_keysym(raw: u32) -> bool {
    matches!(raw, 0xffe1..=0xffee | 0xff7f | 0xfe01..=0xfe13)
}

fn now_msec() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0)
}

/// Set up the active Scaling / Moving grab for the pressed candidate. Mirrors
/// `window.input/input.pointer/window.rs` (`trigger_scale`/`trigger_move`/
/// `trigger_select`), reading windows from `cx.platform.space()`, placeholders
/// from `cx.storage` PLACEHOLDER, and groups inline from `cx.storage` GROUP +
/// space (the Loop-coupled `group_interface::bbox_padded/windows` would cycle).
fn trigger_grab(cx: &mut SystemCx, candidate: &PressCandidate, cursor: Point<f64, Logical>) {
    enum Kind {
        Scale,
        Move,
        Select,
    }
    let kind = match &cx.storage.get(&CANVAS).Grab {
        CanvasGrab::Target(TargetOption::Scale) => Kind::Scale,
        CanvasGrab::Target(TargetOption::Move) => Kind::Move,
        CanvasGrab::Target(TargetOption::Select { .. }) => Kind::Select,
        _ => return,
    };

    if let Kind::Select = kind {
        if let PressCandidate::Window(window) = candidate {
            trigger_select(cx, window.clone());
        }
        return;
    }

    // Scale / Move both build the same candidate-geometry + anchor; Move also
    // accepts a Group ice (Scale does not — `local_type` Group -> None there).
    let allow_group = matches!(kind, Kind::Move);

    let built: Option<(ActiveTransformCandidate, bool, bool)> = match candidate {
        PressCandidate::IcedSurface(handle_id) => match local_type(cx.storage, *handle_id) {
            Some(HandleIdLocalType::Group(group)) if allow_group => {
                let group_windows = group_windows(cx, &group);
                let bbox = group_union_box(cx, &group_windows);
                let (horizontal, vertical) = anchor_flags(bbox, cursor);
                let candidates = trigger_candidates(cx, group_windows);
                Some((ActiveTransformCandidate::Window(candidates), horizontal, vertical))
            }
            Some(HandleIdLocalType::Group(_)) => None,
            Some(HandleIdLocalType::Placeholder) => {
                placeholder_candidate(cx, *handle_id, cursor)
            }
            None => None,
        },
        PressCandidate::Window(window) => {
            let geo = window_box(cx, window);
            let (horizontal, vertical) = anchor_flags(geo, cursor);
            let candidates = trigger_candidates_select(cx, window.clone());
            Some((ActiveTransformCandidate::Window(candidates), horizontal, vertical))
        }
    };

    let Some((candidates, horizontal, vertical)) = built else { return };
    let anchor = Anchor { Horizontal: horizontal, Vertical: vertical };

    // Capture the snap targets ONCE, now that the grab is committing: the edges of
    // every other window + visible placeholder plus the screen edges, excluding the
    // ones being transformed (a source must not snap to its own start edges). The
    // map is frozen for the grab's lifetime; motion only reads it.
    let mut exclude: HashSet<Uuid> = HashSet::new();
    match &candidates {
        ActiveTransformCandidate::Window(list) => {
            for (window, _) in list {
                if let Some(uuid) = window.uuid() {
                    exclude.insert(uuid);
                }
            }
        }
        ActiveTransformCandidate::Placeholder(uuid, _) => {
            exclude.insert(*uuid);
        }
    }
    let snap = build_snap_map(cx, &exclude);

    let grab = match kind {
        Kind::Scale => CanvasGrab::Active(ActiveOption::Scaling {
            candidates,
            start_cursor: cursor,
            Anchor: anchor,
            snap,
        }),
        Kind::Move => CanvasGrab::Active(ActiveOption::Moving {
            candidates,
            start_cursor: cursor,
            Anchor: anchor,
            snap,
        }),
        Kind::Select => unreachable!(),
    };
    cx.write(&CANVAS_BUF, CanvasCmd::SetGrab(grab));
}

fn trigger_select(cx: &mut SystemCx, window: Window) {
    let append = match &cx.storage.get(&CANVAS).Grab {
        CanvasGrab::Target(TargetOption::Select { Append }) => *Append,
        _ => return,
    };
    let select = cx.storage.get(&SELECT);
    let next = if append { select.append(window) } else { select.set(window) };
    announce_selection(cx.channels, SelectionCmd::Set(next));
}

/// Anchor flags from a rect + cursor: horizontal = right half, vertical = bottom
/// half (matches the rim center comparison).
fn anchor_flags(rect: Rectangle<i32, Logical>, cursor: Point<f64, Logical>) -> (bool, bool) {
    let center_x = rect.loc.x as f64 + (rect.size.w as f64 / 2.0);
    let center_y = rect.loc.y as f64 + (rect.size.h as f64 / 2.0);
    (cursor.x >= center_x, cursor.y >= center_y)
}

/// Build the snap map for a grab. `sources` get the rects of every window + every
/// active visible placeholder NOT in `exclude`; the screen edges (output geometry +
/// the camera viewport extent, see [`snap::SNAP_VIEWPORT_EDGES`]) go into the
/// always-on `vertical`/`horizontal` lines. All in storage/world space — the same
/// space the grab geometry math runs in. Window rects use `element_location` +
/// `geometry().size`, matching the start-geo snapshot in `trigger_candidates` so
/// the lines line up with the moving edges.
///
/// When [`snap::SNAP_VISIBLE_ONLY`] is set, sources are culled to those
/// intersecting the current viewport (camera transform + screen size, the same
/// bbox the renderer culls with), inflated by the zoom-scaled
/// [`snap::SNAP_VISIBLE_ONLY_EXTEND_RANGE`] so nearby offscreen windows still count.
fn build_snap_map(cx: &mut SystemCx, exclude: &HashSet<Uuid>) -> SnapMap {
    let mut map = SnapMap::default();

    // Read camera + placeholders from storage BEFORE borrowing `cx.platform`.
    let (camera_pos, camera_zoom) = {
        let camera = cx.storage.get(&OUTPUT_VIEWS).current_views().focus_camera();
        (camera.transform.position, camera.transform.zoom)
    };
    let placeholders: Vec<(Uuid, Rectangle<i32, Logical>)> = cx
        .storage
        .get(&PLACEHOLDER)
        .visible
        .iter()
        .filter(|(ph, _)| ph.restoration.is_none() && !ph.launching)
        .map(|(ph, _)| {
            (
                ph.uuid,
                Rectangle {
                    loc: Point::from((ph.position.0, ph.position.1)),
                    size: Size::from((ph.size.0, ph.size.1)),
                },
            )
        })
        .collect();

    let Some(platform) = cx
        .platform
        .as_deref_mut()
        .and_then(|p| p.downcast_mut::<compositor_orchestration_draw_platform_base::platform::Platform>())
    else {
        return map;
    };
    let space = platform.space();

    // The exact camera viewport sets each source's `visible` flag (whether it is
    // currently on-screen). The CULL viewport (the exact one inflated by the
    // zoom-scaled visible-only extend margin) decides which sources are kept when
    // SNAP_VISIBLE_ONLY is set. `None` cull = keep everything (visible-only off, no
    // output, or an infinite extend range).
    let base_viewport: Option<Rectangle<i32, Logical>> = camera_viewport(space, camera_pos, camera_zoom);
    let cull_viewport: Option<Rectangle<i32, Logical>> = if snap::SNAP_VISIBLE_ONLY {
        match (base_viewport, snap::visible_extend(camera_zoom)) {
            (Some(vp), Some(margin)) => Some(inflate(vp, margin)),
            _ => None,
        }
    } else {
        None
    };
    let keep = |rect: Rectangle<i32, Logical>| -> bool {
        cull_viewport.map(|cv| rect.overlaps(cv)).unwrap_or(true)
    };
    let is_visible = |rect: Rectangle<i32, Logical>| -> bool {
        base_viewport.map(|vp| rect.overlaps(vp)).unwrap_or(true)
    };

    let windows: Vec<Window> = space.elements().cloned().collect();
    for window in &windows {
        if window.uuid().map(|u| exclude.contains(&u)).unwrap_or(false) {
            continue;
        }
        let loc = space.element_location(window).unwrap_or_default();
        let rect = Rectangle { loc, size: window.geometry().size };
        if keep(rect) {
            map.sources.push(SnapSource { rect, visible: is_visible(rect) });
        }
    }

    for (uuid, rect) in placeholders {
        if exclude.contains(&uuid) {
            continue;
        }
        if keep(rect) {
            map.sources.push(SnapSource { rect, visible: is_visible(rect) });
        }
    }

    // The screen-boundary snap lines are the camera VIEWPORT edges — the visible
    // world region (camera-centered, `screen / zoom`), in the same world-logical
    // frame the grab math runs in, so they track pan/zoom and stay on the boundary
    // the user actually sees. (The output geometry is a fixed origin rect that only
    // coincides with the viewport at zoom 1.0 / centered — wrong rect once panned.)
    if snap::SNAP_VIEWPORT_EDGES {
        if let Some(vp) = base_viewport {
            map.vertical.push(vp.loc.x as f64);
            map.vertical.push((vp.loc.x + vp.size.w) as f64);
            map.horizontal.push(vp.loc.y as f64);
            map.horizontal.push((vp.loc.y + vp.size.h) as f64);
        }
    }

    map
}

/// The exact camera viewport in world space: camera-centered, `screen_physical /
/// zoom` (the same bbox the renderer culls with — see `draw.viewport`). `None` when
/// there is no output to size it from.
fn camera_viewport(
    space: &smithay::desktop::Space<Window>,
    camera_pos: Point<f64, Logical>,
    camera_zoom: f64,
) -> Option<Rectangle<i32, Logical>> {
    let mode = space.outputs().next().and_then(|o| o.current_mode())?;
    let logical_w = mode.size.w as f64 / camera_zoom;
    let logical_h = mode.size.h as f64 / camera_zoom;
    Some(Rectangle {
        loc: Point::from(
            ((camera_pos.x - logical_w / 2.0).floor() as i32, (camera_pos.y - logical_h / 2.0).floor() as i32),
        ),
        size: Size::from((logical_w.ceil() as i32, logical_h.ceil() as i32)),
    })
}

/// Inflate a rect outward by `margin` (world units) on every side.
fn inflate(rect: Rectangle<i32, Logical>, margin: f64) -> Rectangle<i32, Logical> {
    let m = margin.ceil() as i32;
    Rectangle {
        loc: Point::from((rect.loc.x - m, rect.loc.y - m)),
        size: Size::from((rect.size.w + 2 * m, rect.size.h + 2 * m)),
    }
}

/// A single window's storage-space rect (`element_location` + the compositor's slot size).
///
/// The size comes from the **decided slot** (`slot::expected_size`), NOT the client's live
/// `geometry()`. This rect seeds the interactive resize/transform (`start_geo` for the delta) and
/// the group anchor center: a `Decided` window whose client committed a different, letterboxed size
/// — e.g. a VM/game that self-sizes — must resize from the compositor's enforced size, otherwise
/// the first motion snaps to `client_size + delta` and jumps. Falls back to `geometry()` only when
/// the slot is unset (matches the pre-slot behaviour for such windows).
fn window_box(cx: &mut SystemCx, window: &Window) -> Rectangle<i32, Logical> {
    let loc = cx
        .platform
        .as_deref_mut()
        .and_then(|p| p.downcast_mut::<compositor_orchestration_draw_platform_base::platform::Platform>())
        .and_then(|p| p.space().element_location(window))
        .unwrap_or_default();
    let size = compositor_y5_camera_transform_translate::slot::expected_size(window)
        .unwrap_or_else(|| window.geometry().size);
    Rectangle { loc, size }
}

/// Inline group inner bbox: the union of group member-window geometries (no
/// padding). The rim's `bbox_padded` pads symmetrically (no center shift) then
/// `pad_y(125)` (asymmetric, shifts the bbox center up ~62.5px) and converts via
/// Transform; the only consumer here is the anchor center, so this approximates
/// the un-padded center — see report.
fn group_union_box(cx: &mut SystemCx, windows: &[Window]) -> Rectangle<i32, Logical> {
    let mut acc: Option<Rectangle<i32, Logical>> = None;
    for w in windows {
        let b = window_box(cx, w);
        acc = Some(match acc {
            Some(a) => a.merge(b),
            None => b,
        });
    }
    acc.unwrap_or_else(|| Rectangle { loc: Point::from((0, 0)), size: Size::from((0, 0)) })
}

/// Group member windows present in the space (mirrors `group_interface::windows`).
fn group_windows(cx: &mut SystemCx, group: &Group) -> Vec<Window> {
    let Some(platform) = cx
        .platform
        .as_deref_mut()
        .and_then(|p| p.downcast_mut::<compositor_orchestration_draw_platform_base::platform::Platform>())
    else {
        return vec![];
    };
    platform
        .space()
        .elements()
        .filter(|w| w.uuid().map(|u| group.window.contains(&u)).unwrap_or(false))
        .cloned()
        .collect()
}

/// Candidate geometries for the trigger window + the current selection (dedup by
/// uuid) — mirrors `get_trigger_candidates_select`.
fn trigger_candidates_select(
    cx: &mut SystemCx,
    trigger: Window,
) -> Vec<(Window, Rectangle<i32, Logical>)> {
    let selection: Vec<Window> = cx
        .storage
        .get(&SELECT)
        .Selection
        .iter()
        .map(|w| w.as_ref().clone())
        .collect();
    let mut windows = vec![trigger];
    windows.extend(selection);
    trigger_candidates(cx, windows)
}

/// Snapshot each window's start geometry, deduped by uuid (mirrors
/// `get_trigger_candidates`).
fn trigger_candidates(
    cx: &mut SystemCx,
    windows: Vec<Window>,
) -> Vec<(Window, Rectangle<i32, Logical>)> {
    let mut candidates: Vec<(Window, Rectangle<i32, Logical>)> = vec![];
    let mut added: HashSet<Uuid> = HashSet::new();
    for window in &windows {
        let Some(uuid) = window.uuid() else { continue };
        if !added.insert(uuid) {
            continue;
        }
        let geo = window_box(cx, window);
        candidates.push((window.clone(), geo));
    }
    candidates
}

/// Placeholder scale/move candidate (mirrors the rim placeholder branch).
fn placeholder_candidate(
    cx: &mut SystemCx,
    handle_id: HandleId,
    cursor: Point<f64, Logical>,
) -> Option<(ActiveTransformCandidate, bool, bool)> {
    let placeholder = cx.storage.get(&PLACEHOLDER).visible.iter().find_map(|w| {
        let active = w.0.restoration.is_none() && w.1.id == handle_id && !w.0.launching;
        if !active {
            return None;
        }
        Some(((w.0.position, w.0.size), w.0.uuid))
    })?;
    let ((position, size), placeholder_uuid) = placeholder;
    let start_geo = Rectangle {
        loc: Point::new(position.0, position.1),
        size: Size::new(size.0, size.1),
    };
    let (horizontal, vertical) = anchor_flags(start_geo, cursor);
    Some((
        ActiveTransformCandidate::Placeholder(placeholder_uuid, start_geo),
        horizontal,
        vertical,
    ))
}

enum HandleIdLocalType {
    Placeholder,
    Group(Group),
}

/// Resolve an iced handle to a placeholder or a group, reading `cx.storage`
/// (mirrors the rim `HandleId::local_type`, which read `&Loop`).
fn local_type(storage: &Storage, handle_id: HandleId) -> Option<HandleIdLocalType> {
    let is_placeholder = storage.get(&PLACEHOLDER).visible.iter().any(|w| {
        w.0.restoration.is_none() && w.1.id == handle_id && !w.0.launching
    });
    if is_placeholder {
        return Some(HandleIdLocalType::Placeholder);
    }
    for item in &storage.get(&GROUP).group {
        if item.Visibility.id() == Some(handle_id) {
            return Some(HandleIdLocalType::Group(item.clone()));
        }
    }
    None
}

/// Window visibility via group state (mirrors `CameraSystem`'s helper / the rim
/// `DrawWindow::visible`): a window in a hidden group is not visible.
fn window_visible(storage: &Storage, window: &Window) -> bool {
    let Some(window_uuid) = window.uuid() else { return true };
    let group_state = storage.get(&GROUP);
    let Some(group_uuid) = group_state.window.get(&window_uuid) else { return true };
    for group in &group_state.group {
        if &group.id != group_uuid.as_ref() {
            continue;
        }
        return matches!(group.Visibility, GroupVisibility::Visible(_));
    }
    false
}

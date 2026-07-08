//! The selection toolbar (align / distribute / stack / scale-to-fit) as an
//! in-process iced surface, reconciled each frame against the live selection.
//!
//! Two placements, chosen by [`SELECTION_OVERLAY_PLACEMENT`]:
//! - `ScreenBottomCenter`: a fixed screen-space bar at the bottom-center.
//! - `WorldAtCursor`: a world-space bar centered just below the cursor and
//!   drawn above all windows. Being world-space, it scales with camera zoom;
//!   its content fills the surface (logical size == dmabuf size).
//!
//! This module owns the surface LIFECYCLE (create/destroy/count) because that
//! needs the `GlesRenderer` (dmabuf alloc), which only the render path has.
//! Re-anchoring on selection change is event-driven and lives in a system —
//! see `compositor_y5_select_overlay_system`.
//!
//! The surface exists only while the selection is non-empty (create/destroy),
//! so when nothing is selected it captures no pointer/keyboard and draws no
//! cursor — there is simply no surface.

use std::process::Command;
use std::sync::Once;

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::Window;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::wayland::seat::WaylandFocus;
use smithay::utils::{Physical, Point, Rectangle, Size};

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_driver_selection_base::base::{
    BAR_H, BAR_W, Placement, SCREEN_BOTTOM_MARGIN, SELECTION_OVERLAY, SELECTION_OVERLAY_MUT,
    SELECTION_OVERLAY_PLACEMENT, SELECTION_REANCHOR_MUT, TIP_GAP, TIP_H, TIP_W, world_loc_under_cursor,
    world_scale_factor, world_size,
};
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_support_world_order_track_base::base::DrawLayer;
use compositor_monitor_compositor_iced_base::{HandleId, IcedHandle, IcedSpace, Transform};
use compositor_monitor_selection_scene_base::selection::{CloseMode, SelectionAction};
use compositor_monitor_selection_scene_base::tip::{TipMessage, TipUi};
use compositor_monitor_selection_scene_base::ui::{Message, Overlay};
use compositor_y5_surface_draw_handle::handle::load;
use compositor_y5_surface_protocol_base::protocol::{
    SelectionForward, SurfaceMessage, SurfaceMessageType,
};
use compositor_remote_message_client_base::bind::selection;

/// True only on the CURSOR's output pass. `per_frame` runs once per output with `render_output`
/// pinned to the output being drawn, and every position path in it (`camera()`, `size_ctx_all()`,
/// the `size` arg) resolves against `render_output`. Running on every pass lets the LAST-drawn
/// monitor's zoom/scale win the stored bar location, while input is hit-tested against the cursor
/// output's camera (`HitCx` in `surface.interface`) — they diverge on multi-monitor and clicks
/// miss. Gating the reconciler to the cursor output puts both sides on the same basis.
/// (`render_output == None` = winit/single pass → act.) Mirrors overview's `on_active_output`.
fn on_active_output(state: &Loop) -> bool {
    state.inner.render_output.as_ref().is_none_or(|k| *k == state.inner.active_output_key())
}

/// Per-frame reconciler (runs from the scene `hooks`, which has the renderer).
/// Creates the toolbar when the selection becomes non-empty (positioned under
/// the cursor for the world placement), destroys it when it empties, and pushes
/// count changes. Reposition-on-change is handled by the system.
pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    // Cursor-output only: otherwise the stored bar position (render-output basis) and the input
    // hit-test (cursor-output basis) resolve against different monitors and clicks miss. See
    // `on_active_output`.
    if !on_active_output(state) {
        return;
    }
    let count = state.inner.select().Selection.len() as i32;
    let handle = state.inner.kernel.get(&SELECTION_OVERLAY).handle;

    match (handle, count) {
        (None, n) if n > 0 => create(state, renderer, size, n),
        (Some(id), 0) => destroy(state, id),
        (Some(id), n) => update(state, id, n),
        (None, _) => {}
    }

    // Apply a re-anchor requested by the selection-change event (system-driven).
    reanchor_if_pending(state);
    // Keep the on-screen size constant as the camera zoom changes.
    resize_on_zoom(state);
    // Show/hide/position the hover tooltip for the currently-hovered button.
    drive_tooltip(state, size);
}

/// Drive the companion tooltip surface each frame: read which button the bar's
/// `Overlay` reports as hovered, and show the separate tip surface (a distinct
/// texture) just above the bar with the matching, modifier-aware text — or hide
/// it when nothing is hovered. Positioned in SCREEN space: for the world-space
/// bar, the bar's stored world location is projected through the camera.
fn drive_tooltip(state: &mut Loop, size: Size<i32, Physical>) {
    let st = state.inner.kernel.get(&SELECTION_OVERLAY);
    let (Some(toolbar_id), Some(tip_id)) = (st.handle, st.tip_handle) else {
        return;
    };
    let last_tip = st.last_tip.clone();

    let scale = state.size_ctx_all().scale;
    let cam = state.inner.camera().transform.clone();
    let cam_t = Transform {
        zoom: cam.zoom,
        position: Point::from((cam.position.x * scale, cam.position.y * scale)),
    };
    let output = size.to_f64();

    let mut new_last_tip = last_tip.clone();

    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        // Visible only while an actual button is hovered (`hovered_tip` is Some).
        let tip = reg
            .get(toolbar_id)
            .and_then(|it| it.get::<Overlay>())
            .and_then(|inst| inst.ui().hovered_tip());

        match tip {
            Some((text, alt, shift)) => {
                let tb = reg.location_of(toolbar_id).unwrap_or_default();
                // Bar's on-screen top-left (project the world-space bar).
                let (sx, sy) = match SELECTION_OVERLAY_PLACEMENT {
                    Placement::ScreenBottomCenter => (tb.x as f64, tb.y as f64),
                    Placement::WorldAtCursor => {
                        let s =
                            cam_t.world_to_screen(output, Point::from((tb.x as f64, tb.y as f64)));
                        (s.x, s.y)
                    }
                };
                // Stuck to the bar's bottom-left: left edge aligned, just below it.
                let pos = Point::from((
                    sx.round() as i32,
                    (sy + (BAR_H as f64) + (TIP_GAP as f64)).round() as i32,
                ));

                let content = (text, alt, shift);
                if last_tip.as_ref() != Some(&content) {
                    let _ = reg.dispatch_message(
                        IcedHandle::<TipUi>::from_id(tip_id),
                        TipMessage::Set {
                            text: content.0.clone(),
                            alt: content.1,
                            shift: content.2,
                        },
                    );
                    new_last_tip = Some(content);
                }
                reg.set_location_by_id(tip_id, pos);
                reg.set_visible_by_id(tip_id, true);
            }
            None => {
                reg.hide_tooltip_by_id(tip_id);
                new_last_tip = None;
            }
        }
    }

    state.inner.kernel.get_mut(&SELECTION_OVERLAY_MUT).last_tip = new_last_tip;
}

/// Counter-scale the world toolbar when zoom changes so it keeps a constant
/// on-screen size (and stays clearly visible when zoomed out). Re-centers on the
/// current world center so it scales in place rather than from its top-left.
fn resize_on_zoom(state: &mut Loop) {
    if SELECTION_OVERLAY_PLACEMENT != Placement::WorldAtCursor {
        return;
    }
    let Some(id) = state.inner.kernel.get(&SELECTION_OVERLAY).handle else {
        return;
    };
    let zoom = state.inner.camera().transform.zoom;
    if state.inner.kernel.get(&SELECTION_OVERLAY).prev_zoom == zoom {
        return;
    }
    let new_size = world_size(zoom);
    let scale = world_scale_factor(zoom);
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let recentered = match (reg.location_of(id), reg.size_of(id)) {
            (Some(loc), Some(old)) => Some(Point::from((
                loc.x + old.w / 2 - new_size.w / 2,
                loc.y + old.h / 2 - new_size.h / 2,
            ))),
            _ => None,
        };
        reg.request_resize_scaled_by_id(id, new_size, scale);
        if let Some(loc) = recentered {
            reg.set_location_by_id(id, loc);
        }
    }
    state.inner.kernel.get_mut(&SELECTION_OVERLAY_MUT).prev_zoom = zoom;
}

/// Consume the one-shot `SELECTION_REANCHOR` flag (raised by the overlay system
/// on a selection-change event) and move the world toolbar under the live
/// cursor. Done here, not in the system, because the cursor comes from the seat.
fn reanchor_if_pending(state: &mut Loop) {
    if SELECTION_OVERLAY_PLACEMENT != Placement::WorldAtCursor {
        return;
    }
    let target = state.inner.worlds.spawn_target();
    let pending = state
        .inner
        .worlds
        .get_mut(target)
        .storage_mut()
        .try_get_mut(&SELECTION_REANCHOR_MUT)
        .map(|flag| std::mem::replace(flag, false))
        .unwrap_or(false);
    if !pending {
        return;
    }
    let Some(id) = state.inner.kernel.get(&SELECTION_OVERLAY).handle else {
        return;
    };
    let loc = world_loc(state);
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.set_location_by_id(id, loc);
    }
}

fn create(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>, count: i32) {
    ensure_font();

    let (loc, sz, space) = placement(state, size);
    let handle = load(
        state,
        renderer,
        Overlay::with_count(count),
        Rectangle::new(loc, sz),
        space,
        Layer::SCENE.bits(),
    );

    // World-space: lift above every window (load registered it at CONTENT).
    if let IcedSpace::World = space {
        state
            .inner
            .register_drawable(uuid::Uuid::from_u128(handle.id.0 as u128), DrawLayer::OVERLAY);
    }

    install_handler(state, handle);

    let untyped = handle.untyped();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        // Give the toolbar iced keyboard focus so Shift/Alt modifiers reach it.
        reg.set_keyboard_focus(Some(untyped));
    }
    // ...and drop the wayland keyboard focus. The keyboard handler routes to iced
    // ONLY when no wayland client is focused (`wayland_handle` short-circuits
    // otherwise), so a focused window would otherwise swallow the modifiers. This
    // mirrors the old layer-shell overlay's on-demand keyboard grab.
    grab_keyboard_to_overlay(state);

    // World placement: set the counter-scale iced factor so content fills the
    // (zoom-counter-scaled) surface. `placement` already sized it for this zoom.
    let zoom = state.inner.camera().transform.zoom;
    if let IcedSpace::World = space {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            reg.request_resize_scaled_by_id(untyped, world_size(zoom), world_scale_factor(zoom));
        }
    }

    // Companion hover-tooltip surface: a separate screen-space, click-through
    // texture that floats above the bar (driven per-frame by `drive_tooltip`).
    // Using its own texture is the whole point of the registry tooltip surface —
    // the tip isn't clipped by the bar's texture and needs no headroom inside it.
    let gpu = state.inner.environment.GPU.clone();
    let tip_id = state
        .inner
        .surface_mut()
        .registry
        .as_mut()
        .and_then(|reg| {
            reg.create_tooltip(
                &gpu.as_str(),
                TipUi::new(),
                renderer,
                Size::from((TIP_W, TIP_H)),
                Layer::SCENE.bits(),
            )
            .ok()
        })
        .map(|h| h.id);

    let st = state.inner.kernel.get_mut(&SELECTION_OVERLAY_MUT);
    st.handle = Some(untyped);
    st.tip_handle = tip_id;
    st.count = count;
    st.prev_zoom = zoom;
}

/// Clear the wayland keyboard focus so keys/modifiers flow to the iced registry
/// (and thus the focused toolbar) instead of a focused window.
fn grab_keyboard_to_overlay(state: &mut Loop) {
    let serial = smithay::utils::SERIAL_COUNTER.next_serial();
    if let Some(keyboard) = state.state.seat.seat.get_keyboard() {
        keyboard.set_focus(
            &mut state.state,
            Option::<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>::None,
            serial,
        );
    }
}

fn update(state: &mut Loop, id: HandleId, count: i32) {
    if state.inner.kernel.get(&SELECTION_OVERLAY).count != count {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            reg.dispatch_message(IcedHandle::<Overlay>::from_id(id), Message::SelectNotify(count));
        }
        state.inner.kernel.get_mut(&SELECTION_OVERLAY_MUT).count = count;
    }
}

fn destroy(state: &mut Loop, id: HandleId) {
    let tip = state.inner.kernel.get(&SELECTION_OVERLAY).tip_handle;
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.destroy_by_id(id); // also clears keyboard focus / pointer / grab
        if let Some(tip) = tip {
            reg.destroy_by_id(tip);
        }
    }
    let st = state.inner.kernel.get_mut(&SELECTION_OVERLAY_MUT);
    st.handle = None;
    st.tip_handle = None;
    st.last_tip = None;
    st.count = 0;
}

/// Drained from the surface message pump (`SurfaceMessageType::Selection`):
/// execute a toolbar action in-process against the canvas.
pub fn handle(state: &mut Loop, _renderer: &mut GlesRenderer, forward: SelectionForward) {
    match forward {
        SelectionForward::Execute(actions, alt) => {
            let request = selection::Layout { action: to_actions(&actions, alt) };
            compositor_remote_client_handle_selection::layout(request, state);
        }
        SelectionForward::ScaleToFit(opt) => {
            compositor_remote_client_handle_aspect::fit_aspect(
                selection::FitAspect {
                    perceived: opt.perceived,
                    max: opt.max,
                    horizontal: opt.horizontal,
                    vertical: opt.vertical,
                },
                state,
            );
        }
        SelectionForward::CloseWindows(mode) => close_selected(state, mode),
    }
}

// --- close selected windows -----------------------------------------------

/// Dismiss every selected window at the chosen [`CloseMode`]. Three strengths:
///
/// - `Request` (no modifier): ask the window to close via the `xdg_toplevel.close`
///   protocol event — the equivalent of clicking its title-bar X. This targets
///   the *surface*, not the process, so windows that share one client process
///   (Chrome's windows, a terminal's windows) close just the chosen one and the
///   app runs its own teardown (save prompts, session save). Windows with no xdg
///   toplevel (XWayland) have no such event, so we fall back to a graceful
///   SIGTERM on the owning pid there.
/// - `Terminate` (Alt): SIGTERM the owning process. The apps are launched by the
///   compositor and best-effort adopted into a transient systemd user `.scope`
///   under `app.slice` (see `introspection.execution.launch`); we prefer
///   `systemctl --user stop <scope>` so the whole cgroup comes down cleanly via
///   systemd's SIGTERM→SIGKILL sequence, falling back to a plain SIGTERM on the
///   pid when the process isn't in such a scope.
/// - `Kill` (Alt+Shift): SIGKILL the pid directly (`kill -9 <pid>`).
///
/// The signal paths target the exact pid that owns the window (resolved from the
/// surface credentials) — never the command line, so other instances of the same
/// app are left alone. All killers are spawned and detached (never waited on) so
/// the render loop is not blocked — the compositor's SIGCHLD reaper collects them.
fn close_selected(state: &Loop, mode: CloseMode) {
    let display_handle = state.inner.loader.display_handle.clone();
    let windows = state.inner.select().Selection.clone();
    for window in &windows {
        // Polite per-surface close: ask the client to dismiss just this window.
        if mode == CloseMode::Request {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_close();
                continue;
            }
            // No xdg toplevel (XWayland): no close event — fall through to a
            // graceful SIGTERM on the owning pid.
        }
        let Some(pid) = window_pid(window, &display_handle) else {
            warn!("close: selected window has no client pid; skipping");
            continue;
        };
        if mode == CloseMode::Kill {
            force_kill(pid);
        } else {
            graceful_close(pid);
        }
    }
}

/// The pid of a window's Wayland client, via the toplevel surface credentials.
fn window_pid(window: &Window, display_handle: &DisplayHandle) -> Option<i32> {
    let surface = window.wl_surface()?;
    let client = surface.client()?;
    client.get_credentials(display_handle).ok().map(|c| c.pid)
}

/// SIGKILL the exact pid that owns the window.
fn force_kill(pid: i32) {
    spawn_detached(Command::new("kill").args(["-9", &pid.to_string()]));
}

/// Gracefully stop the process: `systemctl --user stop <scope>` if it lives in a
/// transient app scope, else SIGTERM the pid directly.
fn graceful_close(pid: i32) {
    match user_scope_of(pid) {
        Some(scope) => {
            spawn_detached(Command::new("systemctl").args(["--user", "stop", &scope]))
        }
        None => spawn_detached(Command::new("kill").args(["-TERM", &pid.to_string()])),
    }
}

/// The leaf `*.scope` unit a pid belongs to, if it sits under `app.slice`
/// (i.e. a window the compositor launched and adopted into systemd). Read from
/// the cgroup-v2 unified line of `/proc/<pid>/cgroup` (`0::<path>`).
fn user_scope_of(pid: i32) -> Option<String> {
    let content = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    for line in content.lines() {
        let Some(path) = line.splitn(3, ':').nth(2) else { continue };
        if !path.contains("app.slice") {
            continue;
        }
        if let Some(leaf) = path.rsplit('/').next() {
            if leaf.ends_with(".scope") {
                return Some(leaf.to_string());
            }
        }
    }
    None
}

/// Spawn a killer command and detach; failures are logged, never fatal.
fn spawn_detached(cmd: &mut Command) {
    if let Err(e) = cmd.spawn() {
        warn!("close: failed to spawn killer: {e}");
    }
}

// --- placement ------------------------------------------------------------

fn placement(
    state: &Loop,
    size: Size<i32, Physical>,
) -> (Point<i32, Physical>, Size<i32, Physical>, IcedSpace) {
    match SELECTION_OVERLAY_PLACEMENT {
        Placement::ScreenBottomCenter => {
            let x = ((size.w - BAR_W) / 2).max(0);
            let y = (size.h - BAR_H - SCREEN_BOTTOM_MARGIN).max(0);
            (Point::from((x, y)), Size::from((BAR_W, BAR_H)), IcedSpace::Screen)
        }
        Placement::WorldAtCursor => {
            // Counter-scaled so the ON-SCREEN size stays constant as zoom changes
            // (a World item's screen size = world size × zoom). The matching iced
            // scale factor (set in `create`/`resize_on_zoom`) keeps content filling.
            let zoom = state.inner.camera().transform.zoom;
            (world_loc(state), world_size(zoom), IcedSpace::World)
        }
    }
}

/// World-physical top-left so the (BAR_W×BAR_H) toolbar is centered horizontally
/// on the cursor and sits just below it. World iced stores location in
/// logical×scale units; the cursor (`pointer.motion`) is logical.
fn world_loc(state: &Loop) -> Point<i32, Physical> {
    // The live cursor in y5-world logical space comes from the seat (the y5
    // PointerState token is not the live value). World iced location is
    // logical × scale.
    let cursor = state
        .state
        .seat
        .seat
        .get_pointer()
        .map(|p| p.current_location())
        .unwrap_or_default();
    let zoom = state.inner.camera().transform.zoom;
    world_loc_under_cursor((cursor.x, cursor.y), state.size_ctx_all().scale, zoom)
}

// --- font / registry plumbing ---------------------------------------------

/// Register the Material Symbols icon font into iced's global font DB once.
fn ensure_font() {
    static ONCE: Once = Once::new();
    ONCE.call_once(compositor_monitor_selection_font_base::font::load);
}

fn install_handler(state: &mut Loop, handle: IcedHandle<Overlay>) {
    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        if let Some(inst) = reg.instance_mut(handle) {
            inst.runtime_mut().set_message_handler(move |m: &Message| {
                let forward = match m {
                    Message::ExecuteSelection(actions, alt) => {
                        Some(SelectionForward::Execute(actions.clone(), *alt))
                    }
                    Message::ExecuteScaleToFit(opt) => Some(SelectionForward::ScaleToFit(*opt)),
                    Message::CloseSelected(mode) => Some(SelectionForward::CloseWindows(*mode)),
                    _ => None,
                };
                if let Some(forward) = forward {
                    let _ = tx.send(SurfaceMessage {
                        message: SurfaceMessageType::Selection(forward),
                    });
                }
            });
        }
    }
}

// --- UI action -> proto layout (mirrors the former gRPC client path) ------

fn to_actions(actions: &[SelectionAction], alternative: bool) -> Vec<selection::Action> {
    actions
        .iter()
        .filter_map(|a| to_action(a, alternative))
        .map(|action| selection::Action { action: Some(action) })
        .collect()
}

fn to_action(a: &SelectionAction, alternative: bool) -> Option<selection::action::Action> {
    let modifier = selection::align::Modifier { stretch: alternative };
    let action = match a {
        SelectionAction::ScaleToFit(_) => return None, // routed via ExecuteScaleToFit
        SelectionAction::AlignTop => selection::action::Action::Align(selection::Align {
            action: Some(selection::align::Action::Top(modifier)),
        }),
        SelectionAction::AlignBottom => selection::action::Action::Align(selection::Align {
            action: Some(selection::align::Action::Bottom(modifier)),
        }),
        SelectionAction::AlignLeft => selection::action::Action::Align(selection::Align {
            action: Some(selection::align::Action::Left(modifier)),
        }),
        SelectionAction::AlignVerticalCenter => {
            selection::action::Action::Align(selection::Align {
                action: Some(selection::align::Action::CenterVertical(modifier)),
            })
        }
        SelectionAction::AlignHorizontalCenter => {
            selection::action::Action::Align(selection::Align {
                action: Some(selection::align::Action::CenterHorizontal(modifier)),
            })
        }
        SelectionAction::AlignRight => selection::action::Action::Align(selection::Align {
            action: Some(selection::align::Action::Right(modifier)),
        }),
        SelectionAction::DistributeHorizontal => {
            selection::action::Action::Distribute(selection::Distribute {
                action: Some(selection::distribute::Action::Horizontal(
                    selection::distribute::Modifier { start: alternative },
                )),
            })
        }
        SelectionAction::DistributeVertical => {
            selection::action::Action::Distribute(selection::Distribute {
                action: Some(selection::distribute::Action::Vertical(
                    selection::distribute::Modifier { start: alternative },
                )),
            })
        }
        SelectionAction::StackHorizontal => selection::action::Action::Stack(selection::Stack {
            action: Some(selection::stack::Action::Horizontal(true)),
        }),
        SelectionAction::StackVertical => selection::action::Action::Stack(selection::Stack {
            action: Some(selection::stack::Action::Vertical(true)),
        }),
    };
    Some(action)
}

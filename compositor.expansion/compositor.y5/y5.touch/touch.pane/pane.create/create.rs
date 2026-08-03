//! Per-frame reconciler for the sticky touch pane. Creates the screen-space iced
//! pane in its summoning world's registry when `inner.touch.pane_world` matches
//! that world, destroys it otherwise, and pushes the live tool-mode to highlight
//! it. Modelled on the FPS overlay (per-world thread_local, `contains`-guarded).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};

use compositor_monitor_compositor_iced_base::{IcedHandle, IcedSpace};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_seat_gesture_touch::touch::TouchMode;
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};
use compositor_y5_touch_pane_view::{PaneMode, TouchPane, TouchPaneMessage};

const PANE_W: i32 = 112;
/// Tall enough for the 9 tap-cells (4 tool-modes + Launcher + Overview + World Picker
/// + OSK + Close) plus their separators; the reconciler centres it and clamps to the
/// output.
const PANE_H: i32 = 880;
/// Gap from the left screen edge (physical px). Kept clear of the edge-swipe
/// band (see `touch/edge.rs` `EDGE_BAND`) so the swipe that hides the pane starts
/// on bare screen to the pane's left, never on a pane button.
const MARGIN: i32 = 56;

thread_local! {
    /// Live pane per (world, output). The iced registry is per-world, but a
    /// Screen-space surface is drawn by EVERY output pass of that world unless it is
    /// output-bound — so, like the FPS overlay, we key by (world uuid, output key),
    /// keep exactly ONE instance (on the active output, bound to it), and tear down
    /// any instance left on a formerly-active output when that output is next drawn.
    static PANE: RefCell<HashMap<(u128, String), (compositor_monitor_compositor_iced_base::HandleId, PaneMode)>> =
        RefCell::new(HashMap::new());
}

fn pane_mode(m: TouchMode) -> PaneMode {
    match m {
        TouchMode::Touch => PaneMode::Touch,
        TouchMode::Pointer => PaneMode::Pointer,
        TouchMode::Hand => PaneMode::Hand,
        TouchMode::Select => PaneMode::Select,
    }
}

/// Left-centre vertical strip.
fn rect(size: Size<i32, Physical>) -> Rectangle<i32, Physical> {
    let y = ((size.h - PANE_H) / 2).max(0);
    Rectangle::new(Point::from((MARGIN, y)), Size::new(PANE_W, PANE_H))
}

/// Reconciler — runs once PER OUTPUT (like the FPS overlay). The pane shows on the
/// ACTIVE monitor only (the one the user is on) and only in its summoning world; on
/// every other output pass this tears down a stray instance keyed to that output.
pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    if state.inner.surface().registry.is_none() {
        return;
    }

    // Fast path for the overwhelmingly common case: the pane was never summoned, so
    // there is nothing to build and nothing keyed to tear down. Checked before the
    // key is assembled because everything below allocates — two `String`s and a
    // hashed tuple lookup — once PER OUTPUT PER FRAME otherwise.
    if state.inner.touch.pane_world.is_none() && PANE.with(|p| p.borrow().is_empty()) {
        return;
    }

    // The output being drawn (`render_output`), falling back to the active-output key
    // on a non-loop / single-output pass so `out == active` holds there.
    let active = state.inner.active_output_key();
    let out = state.inner.render_output.clone().unwrap_or_else(|| active.clone());
    let world = state.inner.worlds.spawn_target().as_u128();
    let key = (world, out.clone());

    // Show ONLY on the active monitor's pass, and only in the world it was summoned
    // in. Any other pass (a different output, or a different world) tears its own
    // keyed instance down — so the pane never lingers on more than one monitor.
    let want = !out.is_empty()
        && out == active
        && state.inner.touch.pane_world == Some(world);

    // Reuse only if the world's registry still holds this key's handle.
    let live = PANE.with(|p| p.borrow().get(&key).copied());
    let live = live.filter(|(h, _)| {
        state.inner.surface().registry.as_ref().is_some_and(|r| r.contains(*h))
    });

    if !want {
        if let Some((id, _)) = live {
            if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                reg.destroy_by_id(id);
            }
            PANE.with(|p| {
                p.borrow_mut().remove(&key);
            });
        }
        return;
    }

    let active_mode = pane_mode(state.inner.touch.tool_mode);
    match live {
        None => {
            if let Some(handle) = create(state, renderer, size, active_mode, &out) {
                PANE.with(|p| {
                    p.borrow_mut().insert(key, (handle.id, active_mode));
                });
            }
        }
        Some((id, shown)) => {
            if shown != active_mode {
                if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                    let _ = reg.dispatch_message(
                        IcedHandle::<TouchPane>::from_id(id),
                        TouchPaneMessage::SetActive(active_mode),
                    );
                }
                PANE.with(|p| {
                    if let Some(e) = p.borrow_mut().get_mut(&key) {
                        e.1 = active_mode;
                    }
                });
            }
        }
    }
}

/// Register the Material Symbols icon font into iced's global DB once (shared
/// with the selection toolbar) so the pane's glyphs render.
fn ensure_font() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(compositor_monitor_selection_font_base::font::load);
}

fn create(
    state: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    active: PaneMode,
    out: &str,
) -> Option<IcedHandle<TouchPane>> {
    ensure_font();
    let mut pane = TouchPane::new();
    pane.active = active;
    let handle = compositor_y5_surface_draw_handle::handle::load(
        state,
        renderer,
        pane,
        rect(size),
        IcedSpace::Screen,
        compositor_orchestration_draw_layer_base::base::Layer::SCENE.bits(),
    );

    // Pin the pane to the output being drawn (`out`) — the scene gate then draws (and
    // hit-tests) this Screen overlay ONLY on that monitor, matching the FPS overlay.
    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    let registry = state.inner.surface_mut().registry.as_mut()?;
    if !out.is_empty() {
        registry.set_output_affinity_by_id(handle.id, Some(out.to_string()));
    }
    registry.set_message_handler(handle, move |m: &TouchPaneMessage| dispatch(m, &tx));
    Some(handle)
}

fn dispatch(message: &TouchPaneMessage, tx: &Sender<SurfaceMessage>) {
    // `SetActive` is compositor → surface only; the rest are actionable taps.
    if matches!(message, TouchPaneMessage::SetActive(_)) {
        return;
    }
    let _ = tx.send(SurfaceMessage {
        message: SurfaceMessageType::TouchPane(message.clone()),
    });
}

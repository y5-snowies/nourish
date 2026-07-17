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
const PANE_H: i32 = 580;
/// Gap from the left screen edge (physical px). Kept clear of the edge-swipe
/// band (see `touch/edge.rs` `EDGE_BAND`) so the swipe that hides the pane starts
/// on bare screen to the pane's left, never on a pane button.
const MARGIN: i32 = 56;

thread_local! {
    /// Live pane per world (the iced registry is per-world): handle + the last
    /// active mode pushed. Keyed by the spawn-target world's uuid, so a world
    /// switch makes a fresh pane in that world's registry.
    static PANE: RefCell<HashMap<u128, (compositor_monitor_compositor_iced_base::HandleId, PaneMode)>> =
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

/// Cursor-output only (mirrors the selection overlay), so the pane is created
/// once rather than once per output pass.
fn on_active_output(state: &Loop) -> bool {
    state
        .inner
        .render_output
        .as_ref()
        .is_none_or(|k| *k == state.inner.active_output_key())
}

/// Left-centre vertical strip.
fn rect(size: Size<i32, Physical>) -> Rectangle<i32, Physical> {
    let y = ((size.h - PANE_H) / 2).max(0);
    Rectangle::new(Point::from((MARGIN, y)), Size::new(PANE_W, PANE_H))
}

pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    if state.inner.surface().registry.is_none() {
        return;
    }
    if !on_active_output(state) {
        return;
    }

    let key = state.inner.worlds.spawn_target().as_u128();
    // Visible only on the world it was summoned in (its own per-world registry
    // holds the surface, so other worlds simply don't have — or show — it).
    let visible = state.inner.touch.pane_world == Some(key);

    // Reuse only if this world's registry still holds the pane's handle.
    let live = PANE.with(|p| p.borrow().get(&key).copied());
    let live = live.filter(|(h, _)| {
        state.inner.surface().registry.as_ref().is_some_and(|r| r.contains(*h))
    });

    if !visible {
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

    let active = pane_mode(state.inner.touch.tool_mode);
    match live {
        None => {
            if let Some(handle) = create(state, renderer, size, active) {
                PANE.with(|p| {
                    p.borrow_mut().insert(key, (handle.id, active));
                });
            }
        }
        Some((id, shown)) => {
            if shown != active {
                if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                    let _ = reg.dispatch_message(
                        IcedHandle::<TouchPane>::from_id(id),
                        TouchPaneMessage::SetActive(active),
                    );
                }
                PANE.with(|p| {
                    if let Some(e) = p.borrow_mut().get_mut(&key) {
                        e.1 = active;
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

    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    let registry = state.inner.surface_mut().registry.as_mut()?;
    registry
        .instance_mut(handle)?
        .runtime_mut()
        .set_message_handler(move |m: &TouchPaneMessage| dispatch(m, &tx));
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

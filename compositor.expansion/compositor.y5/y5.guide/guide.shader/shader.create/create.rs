//! Per-frame reconciler for the inline shader editor against
//! `GuideState::shader_open`, plus the per-frame push of what it displays.
//!
//! SCREEN-space like the help panel, and for a related reason: it is being read
//! and dragged, not pointed at, so it must not scale with the camera. Anchored to
//! the RIGHT edge rather than centred, because the thing it is editing is the
//! whole desktop behind it — a panel in the middle of the screen would cover the
//! effect it exists to let you watch.
//!
//! Pinned to the output being drawn, like the help panel, so a screen overlay does
//! not appear on every monitor at once.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};

use compositor_monitor_compositor_iced_base::{IcedHandle, IcedSpace};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_y5_guide_interface_surface::surface;
use compositor_y5_guide_shader_view::{ShaderEditor, ShaderMessage, ShaderSnapshot};
use compositor_y5_guide_state_base::state::{SHADER_H, SHADER_W};

use std::cell::RefCell;

thread_local! {
    /// Last snapshot pushed into the surface. Re-rendering an iced surface costs a
    /// full relayout and rasterization, and this would otherwise run every frame
    /// the panel is open; the values only move when someone edits them.
    static LAST: RefCell<Option<ShaderSnapshot>> = const { RefCell::new(None) };
}

pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    let want = state.inner.guide().shader_open;
    let live = surface::live(state, |g| g.shader);
    // Teardown runs UNGATED. `destroy_by_id` does not care which output is being
    // drawn, and gating it behind the cursor's output left the panel up whenever
    // that output stopped being drawn (unplug, DPMS, a monitor going idle): the
    // desire was cleared but the only frame allowed to honour it never came.
    // Everything below — creation and the per-frame push — is placement work and
    // does need the gate.
    if !want {
        if let Some(id) = live {
            surface::destroy(state, id);
            state.inner.guide_mut().shader = None;
            LAST.with(|l| *l.borrow_mut() = None);
        }
        return;
    }
    if !surface::on_active_output(state) {
        return;
    }
    match live {
        None => create(state, renderer, size),
        Some(id) => sync(state, id),
    }
}

/// Right-anchored with a margin, clamped to the output so a small monitor still
/// shows the whole panel.
fn rect(size: Size<i32, Physical>) -> Rectangle<i32, Physical> {
    const MARGIN: i32 = 24;
    let (w, h) = (SHADER_W.min(size.w), SHADER_H.min(size.h));
    Rectangle::new(
        Point::from(((size.w - w - MARGIN).max(0), ((size.h - h) / 2).max(0))),
        Size::new(w, h),
    )
}

fn create(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    surface::ensure_font();
    let snapshot = compositor_y5_guide_shader_state::state::snapshot(state);
    LAST.with(|l| *l.borrow_mut() = Some(snapshot.clone()));
    let editor = ShaderEditor { snapshot };
    let handle = compositor_y5_surface_draw_handle::handle::load(
        state, renderer, editor, rect(size), IcedSpace::Screen, Layer::SCENE.bits(),
    );
    let out = state.inner.render_output.clone().unwrap_or_else(|| state.inner.active_output_key());
    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        if !out.is_empty() {
            reg.set_output_affinity_by_id(handle.id, Some(out));
        }
        reg.set_message_handler(handle, move |m: &ShaderMessage| {
            // `Sync` is the compositor's own push coming back around; forwarding it
            // would be a loop. Everything else is an edit and has to leave.
            if matches!(m, ShaderMessage::Sync(_)) {
                return;
            }
            let _ = tx.send(compositor_y5_surface_protocol_base::protocol::SurfaceMessage {
                message: compositor_y5_surface_protocol_base::protocol::SurfaceMessageType::Shader(
                    m.clone(),
                ),
            });
        });
    }
    state.inner.guide_mut().shader = Some(handle.id);
}

/// Push the world's resolved state in, when it has moved.
///
/// The panel does not own these values — the world's `Two` slot does, and the
/// Settings World tab edits the same slot. Re-pushing on change is what keeps the
/// two in step, and what refreshes the whole list when the world's bundle changes
/// under it.
fn sync(state: &mut Loop, id: compositor_monitor_compositor_iced_base::HandleId) {
    let snapshot = compositor_y5_guide_shader_state::state::snapshot(state);
    let changed = LAST.with(|l| {
        let mut l = l.borrow_mut();
        match l.as_ref() == Some(&snapshot) {
            true => false,
            false => {
                *l = Some(snapshot.clone());
                true
            }
        }
    });
    if !changed {
        return;
    }
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let _ = reg.dispatch_message(
            IcedHandle::<ShaderEditor>::from_id(id),
            ShaderMessage::Sync(snapshot),
        );
    }
}

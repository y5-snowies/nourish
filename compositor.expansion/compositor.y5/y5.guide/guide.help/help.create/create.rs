//! Per-frame reconciler for the help panel against `GuideState::help_open`.
//!
//! SCREEN-space and centred, unlike the menu that opened it: this one is being
//! read, not pointed at, so it belongs at a fixed, legible size in the middle of
//! the monitor rather than out on the canvas at whatever the zoom happens to be.
//! Pinned to the output being drawn, like the touch pane, so a screen overlay
//! doesn't appear on every monitor at once.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};

use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_y5_guide_help_view::HelpPanel;
use compositor_y5_guide_interface_surface::surface;
use compositor_y5_guide_state_base::state::{HELP_H, HELP_W};
use compositor_monitor_compositor_iced_base::IcedSpace;

pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    let want = state.inner.guide().help_open;
    let live = surface::live(state, |g| g.help);
    // Teardown is ungated — see the note in `shader.create`: `destroy_by_id` is
    // output-independent, and gating it stranded the panel whenever the cursor's
    // output stopped being drawn.
    if !want {
        if let Some(id) = live {
            surface::destroy(state, id);
            state.inner.guide_mut().help = None;
        }
        return;
    }
    if !surface::on_active_output(state) {
        return;
    }
    if live.is_none() {
        create(state, renderer, size);
    }
}

/// Centred, clamped to the output so a small monitor still shows the whole card.
fn rect(size: Size<i32, Physical>) -> Rectangle<i32, Physical> {
    let (w, h) = (HELP_W.min(size.w), HELP_H.min(size.h));
    Rectangle::new(Point::from((((size.w - w) / 2).max(0), ((size.h - h) / 2).max(0))), Size::new(w, h))
}

fn create(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    surface::ensure_font();
    let panel = HelpPanel {
        rows: compositor_y5_guide_help_row::row::rows(state.inner.storage.nested, &state.inner.keybinding),
    };
    let handle = compositor_y5_surface_draw_handle::handle::load(
        state, renderer, panel, rect(size), IcedSpace::Screen, Layer::SCENE.bits(),
    );
    let out = state.inner.render_output.clone().unwrap_or_else(|| state.inner.active_output_key());
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        if !out.is_empty() {
            reg.set_output_affinity_by_id(handle.id, Some(out));
        }
    }
    state.inner.guide_mut().help = Some(handle.id);
}

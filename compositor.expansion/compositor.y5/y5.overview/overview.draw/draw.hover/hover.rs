//! Overview Layout-tab hover card: the icon + title of the window under the
//! cursor, floating on its grid cell. The content itself comes from
//! `overview.draw/draw.label`; this owns when it shows and where.
//!
//! Resolved per frame from the cell rects the grid recorded (`Overview::cells`),
//! rather than from the input path — the cells are what is actually on screen,
//! and pointer motion already forces a redraw, so this tracks the cursor without
//! the seat knowing the card exists.
//!
//! ## Why the card is never hidden
//!
//! "Nothing hovered" blanks the CONTENT instead of hiding the surface. Hiding
//! makes the worker release the instance's GPU ring (`Job::Visible{false}`), so
//! every crossing between two cells would free three dmabufs and force a full
//! repaint on the way back. Empty content looks identical and costs nothing.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Size};
use uuid::Uuid;
use compositor_monitor_compositor_iced_base::{HandleId, IcedHandle};
use compositor_monitor_overview_ui_hover::{WindowCard, WindowCardMessage};
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_y5_camera_transform_translate::transform::Transform;
use compositor_y5_overview_draw_icon::{icon, IconSource};
use compositor_y5_overview_draw_label::label;
use compositor_y5_overview_state_base::base::MENU_BAR_HEIGHT;

/// Card size in physical px. Fixed: the title is ellipsized to fit rather than
/// the surface being reallocated per window.
const CARD_W: i32 = 320;
const CARD_H: i32 = 40;
/// Inset from the hovered cell's bottom edge.
const CARD_INSET: i32 = 10;

pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    // Once per output in the render loop; the card is a screen-space surface on
    // the ACTIVE monitor only. Mirrors the gate in `draw.status`/`draw.settings`.
    if let Some(k) = &state.inner.render_output {
        if *k != state.inner.active_output_key() {
            return;
        }
    }
    let overview = state.inner.overview();
    if !(overview.visible && overview.overlay_ready()) {
        teardown(state);
        return;
    }
    // Only the window grid has cells to hover.
    if overview.is_world() || overview.is_settings() {
        clear(state);
        return;
    }
    let Some((uuid, cell)) = hovered(state) else {
        clear(state);
        return;
    };
    let Some(window) = label::window_of(state, uuid) else {
        clear(state);
        return;
    };
    // The title is a cheap read and is taken fresh; the icon is not — resolving a
    // name against the icon theme stats its way through every size and context
    // directory, and this runs on every frame the cursor is on a cell. So a
    // RESOLVED icon for the window already on the card is reused. An UNresolved
    // one is retried, which is what lets a late sample (the refresh requested on
    // open) fill the icon in while the cursor is still sitting there — and costs
    // nothing when the window simply has no icon to find.
    let previous = state.inner.overview().card_shown.clone();
    let (resolved, source) = match &previous {
        Some((shown, _, icon @ Some(_))) if *shown == uuid => (icon.clone(), IconSource::Cached),
        _ => icon::resolve(state, &window, uuid),
    };
    let content = (uuid, label::title_of(&window), resolved);
    // Bottom-centred on the cell, then held inside the overlay (a cell can be
    // scrolled half off-screen, and a narrow cell is narrower than the card).
    let at = Point::<i32, Physical>::from((
        (cell.loc.x + (cell.size.w - CARD_W) / 2).clamp(0, (size.w - CARD_W).max(0)),
        (cell.loc.y + cell.size.h - CARD_H - CARD_INSET)
            .clamp(MENU_BAR_HEIGHT, (size.h - CARD_H).max(MENU_BAR_HEIGHT)),
    ));

    let Some(id) = state.inner.overview().card.or_else(|| create(state, renderer)) else {
        return;
    };
    if previous.as_ref() != Some(&content) {
        trace!("hover: {uuid} title={:?} icon from {source:?}", content.1);
        let sent = state.inner.surface_mut().registry.as_mut().is_some_and(|reg| {
            reg.dispatch_message(
                IcedHandle::<WindowCard>::from_id(id),
                WindowCardMessage::Set { title: content.1.clone(), icon: content.2.clone() },
            )
            .is_ok()
        });
        // Only remember what the card was actually told — a dropped message must
        // not leave this claiming content the card never received.
        if sent {
            state.inner.overview_mut().card_shown = Some(content);
        }
    }
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.show_tooltip_by_id(id, at);
    }
}

/// The window whose recorded cell contains the cursor, with that cell.
///
/// The cursor is a y5-WORLD point and the cells are screen/physical, so it is
/// projected the same way the overview's click hit-test projects it.
fn hovered(state: &mut Loop) -> Option<(Uuid, smithay::utils::Rectangle<i32, Physical>)> {
    let location = state.state.seat.seat.get_pointer()?.current_location();
    let ctx = state.size_ctx_all();
    let projected: Transform = (location, ctx).into();
    let physical: Point<f64, Physical> = projected.into();
    let point = Point::<i32, Physical>::from((physical.x.round() as i32, physical.y.round() as i32));
    state
        .inner
        .overview()
        .cells
        .iter()
        .find(|(_, rect)| rect.contains(point))
        .map(|(uuid, rect)| (*uuid, *rect))
}

/// Build the card surface: click-through, and visible for as long as the overlay
/// is up (it draws nothing while blank — see `clear`).
fn create(state: &mut Loop, renderer: &mut GlesRenderer) -> Option<HandleId> {
    let gpu = state.inner.environment.GPU.clone();
    let id = state.inner.surface_mut().registry.as_mut().and_then(|reg| {
        let handle = reg
            .create_screen(
                &gpu.as_str(),
                WindowCard::new(),
                renderer,
                Point::from((0, 0)),
                Size::from((CARD_W, CARD_H)),
                Layer::SCENE.bits(),
            )
            .ok()?;
        // Click-through like a tooltip (`create_tooltip` minus its hidden start
        // — see `clear` for why this one is never hidden).
        reg.set_passthrough_by_id(handle.id, true);
        Some(handle.id)
    })?;
    state.inner.overview_mut().card = Some(id);
    Some(id)
}

/// Blank the card — the cursor is between cells, or on the menu bar.
///
/// Sends EMPTY content rather than hiding the surface: hiding releases the
/// worker's ring for this instance, so re-showing it costs a fresh allocation
/// and a full repaint — on every move between two cells.
fn clear(state: &mut Loop) {
    let Some(id) = state.inner.overview().card else { return };
    if state.inner.overview().card_shown.is_none() {
        return; // already blank
    }
    state.inner.overview_mut().card_shown = None;
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        let _ = reg.dispatch_message(
            IcedHandle::<WindowCard>::from_id(id),
            WindowCardMessage::Set { title: String::new(), icon: None },
        );
    }
}

/// The overlay is down: drop the surface entirely.
pub fn teardown(state: &mut Loop) {
    let overview = state.inner.overview_mut();
    overview.card_shown = None;
    let Some(id) = overview.card.take() else { return };
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.destroy_by_id(id);
    }
}

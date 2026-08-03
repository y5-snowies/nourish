//! Per-frame reconciler for the context menu against `GuideState::menu_at`, plus
//! its companion tooltip surface.
//!
//! World-ANCHORED but zoom-locked: the pill rides the canvas from where it was
//! summoned, yet is drawn at exactly `MENU_W`×`MENU_H` screen pixels forever
//! (`set_zoom_lock_by_id`), from a buffer supersampled `MENU_SUPERSAMPLE` times
//! that. Only the anchor is re-derived on zoom — no dmabuf is ever reallocated.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Rectangle, Size};

use compositor_monitor_compositor_iced_base::{HandleId, IcedSpace};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_support_world_order_track_base::base::DrawLayer;
use compositor_y5_guide_interface_surface::surface;
use compositor_y5_guide_menu_tip::GuideTip;
use compositor_y5_guide_menu_view::{GuideMenu, GuideMessage};
use compositor_y5_guide_state_base::state::{menu_buffer, world_loc, GUIDE, GUIDE_MUT, MENU_SUPERSAMPLE, TIP_H, TIP_W};

pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer) {
    let want = state.inner.kernel.get(&GUIDE).menu_at;
    let live = surface::live(state, |g| g.menu);
    if want.is_none() && live.is_none() {
        return;
    }
    if !surface::on_active_output(state) {
        return;
    }
    match (want, live) {
        (Some(at), None) => create(state, renderer, at),
        (Some(_), Some(id)) => reanchor(state, id),
        (None, Some(id)) => destroy(state, id),
        (None, None) => {}
    }
}

fn create(state: &mut Loop, renderer: &mut GlesRenderer, at: (f64, f64)) {
    surface::ensure_font();
    let nested = state.inner.storage.nested;
    let scale = state.size_ctx_all().scale;
    let zoom = state.inner.camera().transform.zoom;
    let menu = GuideMenu {
        hovered: None,
        settings_hint: compositor_y5_guide_help_row::row::settings_hint(nested),
        help_hint: "Help".to_string(),
    };
    // The dmabuf is supersampled; `set_zoom_lock` below divides it back down to
    // MENU_W×MENU_H on screen, so the layout and the anchor are unaffected.
    let size = menu_buffer();
    let rect = Rectangle::new(world_loc(at, scale, zoom), size);
    let handle = compositor_y5_surface_draw_handle::handle::load_snapshot(state, renderer, menu, rect, IcedSpace::World, Layer::SCENE.bits());
    // World-space: lift above every window (`load_snapshot` registered it at CONTENT).
    state.inner.register_drawable(uuid::Uuid::from_u128(handle.id.0 as u128), DrawLayer::OVERLAY);
    let tx = state.inner.surface_mut().surface_message_buffer_channel.0.clone();
    let gpu = state.inner.environment.GPU.clone();
    let tip = state.inner.surface_mut().registry.as_mut().and_then(|reg| {
        reg.set_message_handler(handle, move |m: &GuideMessage| surface::dispatch(m, &tx));
        reg.set_zoom_lock_by_id(handle.id, Some(MENU_SUPERSAMPLE));
        // Matching iced factor: the UI still lays out at MENU_W×MENU_H logical,
        // it is just rasterized into the larger buffer.
        reg.request_resize_scaled_by_id(handle.id, size, MENU_SUPERSAMPLE);
        reg.create_tooltip(&gpu.as_str(), GuideTip::new(), renderer, Size::from((TIP_W, TIP_H)), Layer::SCENE.bits())
            .ok()
            .map(|h| h.id)
    });
    let guide = state.inner.kernel.get_mut(&GUIDE_MUT);
    guide.menu = Some(handle.id);
    guide.tip = tip;
    guide.menu_zoom = zoom;
}

fn destroy(state: &mut Loop, id: HandleId) {
    surface::destroy(state, id);
    if let Some(tip) = state.inner.kernel.get(&GUIDE).tip {
        surface::destroy(state, tip);
    }
    let guide = state.inner.kernel.get_mut(&GUIDE_MUT);
    guide.menu = None;
    guide.tip = None;
    guide.last_tip = None;
}

/// Re-derive the world location when the zoom changes. The pill is centred on a
/// WORLD anchor but is a fixed number of SCREEN pixels wide, so its half-width in
/// world units is `MENU_W/2/zoom` — zoom-dependent even though the size is not.
/// Location only: no resize, no reallocation.
fn reanchor(state: &mut Loop, id: HandleId) {
    let zoom = state.inner.camera().transform.zoom;
    let guide = state.inner.kernel.get(&GUIDE);
    let Some(at) = guide.menu_at.filter(|_| guide.menu_zoom != zoom) else { return };
    let scale = state.size_ctx_all().scale;
    let loc = world_loc(at, scale, zoom);
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        reg.set_location_by_id(id, loc);
    }
    state.inner.kernel.get_mut(&GUIDE_MUT).menu_zoom = zoom;
}

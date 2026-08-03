//! Drives the guide menu's tooltip each frame: read which entry the menu
//! reports as hovered, then show the separate tip surface just below the menu
//! with that entry's shortcut — or hide it when nothing is hovered.
//!
//! Positioned in SCREEN space, so the menu's stored WORLD location is projected
//! through the camera first. Same shape as the selection toolbar's
//! `drive_tooltip`, which is the convention every floating hint here follows.

use smithay::utils::{Physical, Point, Size};

use compositor_monitor_compositor_iced_base::{IcedHandle, Transform};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;
use compositor_y5_guide_menu_tip::{GuideTip, GuideTipMessage};
use compositor_y5_guide_menu_view::GuideMenu;
use compositor_y5_guide_state_base::state::{GUIDE, GUIDE_MUT, MENU_H, TIP_GAP, TIP_W};

pub fn per_frame(state: &mut Loop, size: Size<i32, Physical>) {
    let guide = state.inner.kernel.get(&GUIDE);
    let (Some(menu), Some(tip)) = (guide.menu, guide.tip) else { return };
    let last = guide.last_tip.clone();

    let scale = state.size_ctx_all().scale;
    let camera = state.inner.camera().transform.clone();
    let projected = Transform {
        zoom: camera.zoom,
        position: Point::from((camera.position.x * scale, camera.position.y * scale)),
    };
    let output = size.to_f64();

    let mut next = last.clone();
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        // Published copy rather than a borrow — see `IcedSnapshot`. Doubly
        // wrapped: outer `None` = no such instance, inner = nothing hovered.
        match reg.snapshot::<GuideMenu>(IcedHandle::from_id(menu)).flatten() {
            Some(label) => {
                let at = reg.location_of(menu).unwrap_or_default();
                let top_left = projected.world_to_screen(output, Point::from((at.x as f64, at.y as f64)));
                // Centred on the menu, hanging just below it. The menu's own
                // on-screen width is constant (it is counter-scaled), so the
                // stored MENU_W is the right half-width to centre against.
                let pos = Point::from((
                    (top_left.x + (compositor_y5_guide_state_base::state::MENU_W as f64) / 2.0 - (TIP_W as f64) / 2.0).round() as i32,
                    (top_left.y + (MENU_H + TIP_GAP) as f64).round() as i32,
                ));
                if last.as_ref() != Some(&label) {
                    let _ = reg.dispatch_message(IcedHandle::<GuideTip>::from_id(tip), GuideTipMessage::Set(label.clone()));
                    next = Some(label);
                }
                reg.set_location_by_id(tip, pos);
                reg.set_visible_by_id(tip, true);
            }
            None => {
                reg.hide_tooltip_by_id(tip);
                next = None;
            }
        }
    }
    state.inner.kernel.get_mut(&GUIDE_MUT).last_tip = next;
}

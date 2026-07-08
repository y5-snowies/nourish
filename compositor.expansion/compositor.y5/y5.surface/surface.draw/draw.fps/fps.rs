use iced_core::{alignment, Background, Color, Element, Length, Theme};
use iced_widget::{column, container, text};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Point, Rectangle, Size};
use std::cell::RefCell;
use std::collections::HashMap;

use compositor_monitor_compositor_iced_base::{HandleId, IcedHandle, IcedSpace};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_draw_layer_base::base::Layer;
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use compositor_y5_surface_draw_handle::handle::load;

/// Overlay box size (physical px) and its inset from the top-right corner.
const W: i32 = 120;
const H: i32 = 44;
const MARGIN: i32 = 12;

/// The overlay's iced UI: the last measured FPS.
#[derive(Default)]
pub struct FpsUi {
    fps: u32,
}

#[derive(Debug, Clone)]
pub enum FpsMessage {
    Set { fps: u32 },
}

impl IcedUi for FpsUi {
    type Message = FpsMessage;

    fn update(&mut self, message: FpsMessage) {
        let FpsMessage::Set { fps } = message;
        self.fps = fps;
    }

    fn view(&self) -> Element<'_, FpsMessage, Theme, Renderer> {
        let body = column![text(format!("{} FPS", self.fps)).size(24).color(Color::WHITE)];
        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(8)
            .align_x(alignment::Horizontal::Left)
            .align_y(alignment::Vertical::Center)
            .style(|_t| container::Style {
                background: Some(Background::Color(Color::BLACK)),
                ..Default::default()
            })
            .into()
    }
}

/// One overlay surface, living in one world's registry.
struct Overlay {
    handle: HandleId,
    sized: (i32, i32),
    shown: Option<u32>,
}

thread_local! {
    /// Overlay per (world, output). The iced registry is PER WORLD, so each world
    /// the user visits needs its own overlay in its own registry — keyed by the
    /// spawn-target world's uuid so a world switch creates a fresh one instead of
    /// reusing a handle from another world's registry.
    static OVERLAYS: RefCell<HashMap<(u128, String), Overlay>> = RefCell::new(HashMap::new());
}

/// Top-right rect at the given output size (output-local coords).
fn fps_rect(size: Size<i32, Physical>) -> Rectangle<i32, Physical> {
    Rectangle::from_loc_and_size(
        Point::from(((size.w - W - MARGIN).max(0), MARGIN)),
        Size::from((W, H)),
    )
}

/// Tear down any overlays that live in the CURRENT world's registry (preference
/// turned off). Overlays in other worlds' registries are removed when those
/// worlds are next drawn with the preference off.
fn teardown(state: &mut Loop) {
    if OVERLAYS.with(|o| o.borrow().is_empty()) {
        return;
    }
    if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
        OVERLAYS.with(|o| {
            o.borrow_mut().retain(|_, ov| {
                if reg.contains(ov.handle) {
                    reg.destroy_by_id(ov.handle);
                    false
                } else {
                    true
                }
            });
        });
    }
}

/// Per-frame hook, run once PER OUTPUT inside the GLES prepare pass (each call is
/// that output's vblank-paced draw). Gated by the `show_fps` preference; keeps a
/// per-monitor, per-world overlay showing that output's draw rate.
pub fn per_frame(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) {
    // Registry is created after startup; nothing to draw into before then.
    if state.inner.surface().registry.is_none() {
        return;
    }
    // Preference gate (Settings → Performance). Off ⇒ ensure nothing is shown.
    if !state.inner.preference.show_fps {
        teardown(state);
        return;
    }

    // Output being drawn (winit / single-output ⇒ one default key) and the world
    // whose registry `surface()` resolves to (the focused spawn target).
    let out = state.inner.render_output.clone().unwrap_or_default();
    let world = state.inner.worlds.spawn_target().as_u128();
    let key = (world, out.clone());

    // The true per-output present rate — computed kernel-side at page-flip
    // completion / winit present, so dropped frames show as a lower number.
    let fps = compositor_developer_stats_registry_base::base::present_rate(&out);

    let rect = fps_rect(size);
    // Reuse only if this world's registry still holds the overlay's handle.
    let live = OVERLAYS.with(|o| o.borrow().get(&key).map(|ov| (ov.handle, ov.sized)));
    let live = live.filter(|(h, _)| {
        state
            .inner
            .surface()
            .registry
            .as_ref()
            .is_some_and(|r| r.contains(*h))
    });

    match live {
        None => {
            // Create in THIS world's registry, bound to this output.
            let handle = load(
                state,
                renderer,
                FpsUi::default(),
                rect,
                IcedSpace::Screen,
                Layer::SCENE.bits(),
            );
            if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                reg.set_passthrough_by_id(handle.id, true);
                if !out.is_empty() {
                    reg.set_output_affinity_by_id(handle.id, Some(out.clone()));
                }
            }
            OVERLAYS.with(|o| {
                o.borrow_mut().insert(
                    key.clone(),
                    Overlay {
                        handle: handle.id,
                        sized: (size.w, size.h),
                        shown: None,
                    },
                );
            });
        }
        Some((id, sized)) => {
            if sized != (size.w, size.h) {
                if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
                    reg.set_location_by_id(id, rect.loc);
                }
                OVERLAYS.with(|o| {
                    if let Some(ov) = o.borrow_mut().get_mut(&key) {
                        ov.sized = (size.w, size.h);
                    }
                });
            }
        }
    }

    // Push a new value only when it changed, so an unchanging overlay stays idle.
    let push = OVERLAYS.with(|o| {
        let mut map = o.borrow_mut();
        map.get_mut(&key).and_then(|ov| {
            (ov.shown != Some(fps)).then(|| {
                ov.shown = Some(fps);
                ov.handle
            })
        })
    });
    if let Some(id) = push {
        if let Some(reg) = state.inner.surface_mut().registry.as_mut() {
            let _ = reg.dispatch_message(IcedHandle::<FpsUi>::from_id(id), FpsMessage::Set { fps });
        }
    }
}

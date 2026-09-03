use smithay::desktop::{Space, Window};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Rectangle, Logical};
use smithay::wayland::{compositor, fractional_scale};
use smithay::wayland::compositor::{send_surface_state, with_states};
use compositor_support_smithay_state_fractional_base::state::NestedCompositorSurface;
use compositor_support_smithay_state_window_find::find;

/// The parent window's rect in `space` — its location and its own geometry size.
///
/// Single-world by construction: the caller chooses the Space. Everything on the commit
/// path wants `owning_space`, not the focused one — a commit belongs to its window's
/// world, which is not necessarily the world the user is looking at.
pub fn parent_geometry(space: &Space<Window>, parent: &WlSurface) -> Rectangle<i32, Logical> {
    find::in_space(space, parent)
        .map(|w| {
            let loc = space.element_location(&w).unwrap_or_default();
            Rectangle::from_loc_and_size(loc, w.geometry().size)
        })
        .unwrap_or_default()
}

pub fn new_fractional_scale(scale: f64, surface: &WlSurface) {
    let integer_scale = scale.ceil() as i32;
    compositor::with_states(surface, |states| {
        if states.data_map.get::<NestedCompositorSurface>().is_some() { return; }
        fractional_scale::with_fractional_scale(states, |fs| { fs.set_preferred_scale(scale); });
        send_surface_state(surface, states, integer_scale, smithay::utils::Transform::Normal);
    });
}

pub fn new_surface_fractional(scale: f64, surface: &WlSurface) {
    let integer_scale = scale.ceil() as i32;
    with_states(surface, |states| {
        send_surface_state(surface, states, integer_scale, smithay::utils::Transform::Normal);
    });
}

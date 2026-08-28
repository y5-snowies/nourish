use smithay::desktop::{Space, Window};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::seat::WaylandFocus;
use smithay::utils::{Rectangle, Logical};
use smithay::wayland::{compositor, fractional_scale};
use smithay::wayland::compositor::{send_surface_state, with_states};
use compositor_support_smithay_state_fractional_base::state::NestedCompositorSurface;

pub fn window_for_toplevel(space: &Space<Window>, surface: &WlSurface) -> Option<Window> {
    space.elements()
        .find(|w| w.toplevel().map(|t| t.wl_surface() == surface).unwrap_or(false))
        .cloned()
}

pub fn parent_geometry(space: &Space<Window>, parent: &WlSurface) -> Rectangle<i32, Logical> {
    space.elements()
        .find(|w| w.wl_surface().map(|s| s.as_ref() == parent).unwrap_or(false))
        .map(|w| {
            let loc = space.element_location(w).unwrap_or_default();
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

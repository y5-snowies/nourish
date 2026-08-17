use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{self, TraversalAction, send_surface_state, with_surface_tree_downward};
use smithay::wayland::fractional_scale;

pub struct NestedCompositorSurface {}

/// Push `scale` to every surface in the iterator, and to each one's SUBSURFACES.
/// Surfaces without a fractional scale object still get `wl_surface` state; ones marked
/// [`NestedCompositorSurface`] are skipped entirely.
///
/// The tree walk matters as much as the roots. `preferred_scale` and
/// `preferred_buffer_scale` are per-`wl_surface`, and a client puts real content in
/// subsurfaces — CSD decorations, a video frame, an overlay. The caller can only ever hand
/// us window ROOTS, because a zoom belongs to a window, so publishing to just those left
/// every subsurface pinned to whatever scale existed when it was created (`new_surface`
/// seeds one, and nothing updated it afterwards). Its content then stayed at that scale
/// while the toplevel followed the zoom, going progressively softer against a sharp window.
pub fn emit_to_surfaces<'a, I>(scale: f64, surfaces: I)
where
    I: IntoIterator<Item = &'a WlSurface>,
{
    let integer_scale = scale.ceil() as i32;
    let transform = smithay::utils::Transform::Normal;

    for surface in surfaces {
        with_surface_tree_downward(
            surface,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |surface, states, _| {
                if states.data_map.get::<NestedCompositorSurface>().is_some() {
                    return;
                }
                fractional_scale::with_fractional_scale(states, |fs| {
                    fs.set_preferred_scale(scale);
                });

                send_surface_state(surface, states, integer_scale, transform);
            },
            |_, _, _| true,
        );
    }
}

/// Kept so callers that genuinely mean "this surface alone" stay explicit about it.
pub fn emit_to_surface(scale: f64, surface: &WlSurface) {
    let integer_scale = scale.ceil() as i32;
    compositor::with_states(surface, |states| {
        if states.data_map.get::<NestedCompositorSurface>().is_some() {
            return;
        }
        fractional_scale::with_fractional_scale(states, |fs| {
            fs.set_preferred_scale(scale);
        });
        send_surface_state(surface, states, integer_scale, smithay::utils::Transform::Normal);
    });
}

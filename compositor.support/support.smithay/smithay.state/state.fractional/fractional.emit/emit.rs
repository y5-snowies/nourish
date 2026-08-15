use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{self, send_surface_state};
use smithay::wayland::fractional_scale;

pub struct NestedCompositorSurface {}

/// Push `scale` to every surface in the iterator that has a fractional
/// scale object. Surfaces without one are silently ignored.
pub fn emit_to_surfaces<'a, I>(scale: f64, surfaces: I)
where
    I: IntoIterator<Item = &'a WlSurface>,
{
    let integer_scale = scale.ceil() as i32;
    let transform = smithay::utils::Transform::Normal;

    for surface in surfaces {
        compositor::with_states(surface, |states| {
            if states.data_map.get::<NestedCompositorSurface>().is_some() {
                return;
            }
            fractional_scale::with_fractional_scale(states, |fs| {
                fs.set_preferred_scale(scale);
            });

            send_surface_state(surface, states, integer_scale, transform);
        });
    }
}

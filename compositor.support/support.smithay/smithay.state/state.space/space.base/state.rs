use smithay::desktop::{Space, Window};
use smithay::output::{Output, Scale};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use smithay::wayland::seat::WaylandFocus;

pub struct SpaceState {
    // A `Space` represents a two-dimensional coordinate plane. Windows and Outputs (monitors)
    // are mapped onto it.
    //
    // **Side-Effects/Redraws:** `Space` doesn't directly talk to `calloop`. Instead, at the end of
    // every `calloop` tick (or tied to monitor vblank), you will call `space.render_output()`.
    // This traverses the 2D plane, computes damage (what changed), and physically redraws the screen.
    pub state: Space<Window>,
}

impl SpaceState {
    pub fn default_logical(&self) -> Rectangle<i32, Logical> {
        let compositor_output = self.state.outputs().next().unwrap();
        let compositor_output_geometry = self.state.output_geometry(compositor_output).unwrap();
        return compositor_output_geometry;
    }

    pub fn default_physical_i32(&self) -> Rectangle<i32, Physical> {
        let compositor_output = self.state.outputs().next().unwrap();
        let compositor_output_geometry = self.state.output_geometry(compositor_output).unwrap();

        let scale = compositor_output.current_scale();

        return compositor_output_geometry.to_physical(scale.integer_scale());
    }

    pub fn default_physical_precise(&self) -> Rectangle<f64, Physical> {
        let compositor_output = self.state.outputs().next().unwrap();
        let compositor_output_geometry = self.state.output_geometry(compositor_output).unwrap();

        let scale = compositor_output.current_scale();

        return compositor_output_geometry.to_physical_precise_round(scale.integer_scale());
    }

    pub fn default_scale(&self) -> Scale {
        let compositor_output = self.state.outputs().next().unwrap();
        let compositor_output_geometry = self.state.output_geometry(compositor_output).unwrap();

        let scale = compositor_output.current_scale();
        scale
    }

    pub fn default_output(&self) -> &Output {
        let compositor_output = self.state.outputs().next().unwrap();
        compositor_output
    }

    pub fn default_output_geometry(&self) -> Rectangle<i32, Logical> {
        let compositor_output = self.state.outputs().next().unwrap();
        let compositor_output_geometry = self.state.output_geometry(compositor_output).unwrap();
        compositor_output_geometry
    }

    pub fn default_scale_logical(&self) -> f64 {
        let compositor_output = self.state.outputs().next().unwrap();
        let compositor_output_geometry = self.state.output_geometry(compositor_output).unwrap();

        let scale = compositor_output.current_scale();
        scale.fractional_scale()
    }

    pub fn element_location_for_surface(&self, hint_surface: &WlSurface) -> Point<i32, Logical> {
        self.state
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(hint_surface))
            .and_then(|w| self.state.element_location(w))
            .unwrap_or_default()
    }
}

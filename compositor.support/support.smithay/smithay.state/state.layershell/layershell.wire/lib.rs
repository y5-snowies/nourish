#[macro_use]
extern crate compositor_developer_debug_instance_record;

use smithay::desktop::{layer_map_for_output, LayerSurface as SmithayLayerSurface, WindowSurfaceType};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::shell::wlr_layer::{Layer, LayerSurface, WlrLayerShellState};
use compositor_support_smithay_dispatch_state_base::state::{Dispatch, DispatchWire};

pub fn shell_state(dispatch: &mut Dispatch) -> &mut WlrLayerShellState {
    &mut dispatch.layershell.wlr
}

pub fn new_layer_surface(
    space: &compositor_support_smithay_state_space_base::state::SpaceState,
    surface: LayerSurface,
    output: Option<WlOutput>,
    _layer: Layer,
    namespace: String,
    // The monitor the user is currently on — used when the client passes NULL
    // output (spec: compositor picks, conventionally the focused monitor). Falls
    // back to the first output only if no current output is resolvable.
    current_output: Option<Output>,
) {
    info!("new layer surface namespace={namespace:?}");

    let target_output: Option<Output> = output
        .as_ref()
        .and_then(|wl_out| Output::from_resource(wl_out))
        .or(current_output)
        .or_else(|| space.state.outputs().next().cloned());

    let Some(output) = target_output else {
        warn!("no output available for layer surface; closing, {:?}", output);
        return;
    };

    let desktop_layer = SmithayLayerSurface::new(surface.clone(), namespace);
    let mut layer_map = layer_map_for_output(&output);
    layer_map
        .map_layer(&desktop_layer)
        .unwrap_or_else(|e| abort!("failed to map layer surface: {e:?}"));
    drop(layer_map);

    // `map_layer` runs smithay's `arrange()`, which computes this surface's geometry from
    // the client's anchor / requested size / margin / exclusive-zone and stores it (read
    // back via `LayerMap::layer_geometry`, used by the draw + hit paths). `arrange()`
    // deliberately does NOT send the INITIAL configure (the spec requires it in response to
    // the first commit), so we send it here — carrying the anchored size arrange just set
    // on the pending state. A dimension the client left 0 stays 0 (its choice / stretch).
    surface.send_configure();
}

/// Re-run smithay's layer arrangement for whichever output owns `surface`, if it is a
/// mapped layer surface. Called on every commit so a client's LIVE anchor / size / margin /
/// exclusive-zone change repositions the surface and re-reserves exclusive space. Returns
/// whether a layer was re-arranged (so the caller can schedule a redraw).
pub fn arrange_on_commit(
    space: &compositor_support_smithay_state_space_base::state::SpaceState,
    surface: &WlSurface,
) -> bool {
    let mut arranged = false;
    for output in space.state.outputs() {
        let mut layer_map = layer_map_for_output(output);
        // `layer_for_surface` borrows the map immutably; `is_some()` drops that borrow
        // before we take the mutable `arrange()` borrow.
        if layer_map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).is_some() {
            layer_map.arrange();
            arranged = true;
        }
    }
    arranged
}

pub fn layer_destroyed(space: &compositor_support_smithay_state_space_base::state::SpaceState, surface: LayerSurface) {
    for output in space.state.outputs() {
        let mut layer_map = layer_map_for_output(output);
        let to_unmap = layer_map
            .layers()
            .find(|l| l.layer_surface() == &surface)
            .cloned();
        if let Some(desktop_layer) = to_unmap {
            layer_map.unmap_layer(&desktop_layer);
        }
    }
}

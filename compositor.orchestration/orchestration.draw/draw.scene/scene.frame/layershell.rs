//! Layer-shell contribution as renderer-agnostic draw nodes (goal B): we carry
//! each layer's `wl_surface` + physical placement; the backend builds the
//! `WaylandSurfaceRenderElement`s at lowering time. No renderer here.

use smithay::desktop::{layer_map_for_output, PopupManager};
use smithay::utils::{Physical, Size};
use smithay::wayland::shell::wlr_layer::Layer;
use compositor_orchestration_draw_node_base::node::SurfaceNode;
use compositor_orchestration_core_state_base::Loop;

/// Each node carries its wlr `Layer` so the caller can slot it into the matching
/// render band (LAYER_BACKGROUND/BOTTOM below content, LAYER_TOP/OVERLAY above) —
/// keeping draw z-order consistent with hit-testing rather than lumping every
/// layer above windows.
pub fn layershell(
    state: &mut Loop,
    _size: Size<i32, Physical>,
    render_key: Option<&str>,
) -> Vec<(Layer, SurfaceNode)> {
    let mut nodes = vec![];

    for output in state.inner.space_state().state.outputs() {
        // A layer surface is OUTPUT-BOUND: it lives in exactly one output's layer map, at
        // that output's LOCAL coords. scene() runs once per physical output, so gather
        // ONLY the output being drawn — otherwise every monitor also draws the OTHER
        // monitors' bars (at foreign local coords), the duplicated/mispositioned artifact.
        // `None` = winit / single-output pass → gather all (there is only one).
        if let Some(key) = render_key {
            if compositor_orchestration_core_state_base::state::output_key(output).as_str() != key {
                continue;
            }
        }
        let scale = output.current_scale().fractional_scale();
        let layer_map = layer_map_for_output(output);

        for layer in layer_map.layers().rev() {
            // smithay's `arrange()` geometry — honors anchor / margin / exclusive-zone /
            // size (logical, output-relative). Convert to physical for the draw node.
            let Some(geo) = layer_map.layer_geometry(layer) else {
                continue;
            };
            let band = layer.layer();
            nodes.push((
                band,
                SurfaceNode {
                    surface: layer.wl_surface().clone(),
                    location: geo.loc.to_f64().to_physical(scale).to_i32_round(),
                    scale,
                    alpha: 1.0,
                },
            ));

            // Popups parented to the layer surface (e.g. a menu off a panel/bar). Their
            // reported location is relative to the parent surface origin; smithay's render
            // convention is `parent_loc + popup_loc - popup.geometry().loc`. Rendered in
            // the SAME band as the parent so a Top/Overlay menu stacks with its bar.
            for (popup, popup_loc) in PopupManager::popups_for_surface(layer.wl_surface()) {
                let offset = popup_loc - popup.geometry().loc;
                nodes.push((
                    band,
                    SurfaceNode {
                        surface: popup.wl_surface().clone(),
                        location: (geo.loc + offset).to_f64().to_physical(scale).to_i32_round(),
                        scale,
                        alpha: 1.0,
                    },
                ));
            }
        }
    }

    nodes
}

use smithay::utils::{Logical, Rectangle};
use compositor_support_action_camera_find_window::{WindowEntry, WindowId};
use compositor_support_action_camera_find_flags::WindowFinderFlags;
use compositor_support_action_camera_find_direction::Direction;
use compositor_support_action_camera_find_axes::DirAxes;
use compositor_support_action_camera_find_passes::{EndpointPass, PassResult};
use compositor_support_action_camera_find_band::BandState;
use compositor_support_action_camera_find_origin::synthesize_origin;
use compositor_support_action_camera_find_pick::pick_origin;
use compositor_support_action_camera_find_build::build_base_passes;
use compositor_support_action_camera_find_apply::apply_base_pass;
use compositor_support_action_camera_find_endpoint::compute_endpoint;
use compositor_support_action_camera_find_ray::{cast_ray, hits_bounding_box};
use compositor_support_action_camera_find_sort::sort_results;

pub fn find(
    flags: WindowFinderFlags,
    dir: Direction,
    windows: &[WindowEntry],
    outputs: &[Rectangle<f64, Logical>],
    focused: Option<WindowId>,
) -> Vec<PassResult> {
    debug_assert!(!outputs.is_empty(), "find() requires at least one output");
    if outputs.is_empty() { return Vec::new(); }

    let axes = DirAxes { dir };
    let synthetic_holder;
    let origin: &WindowEntry = match pick_origin(flags, focused, windows, outputs, &axes) {
        Some(id) => windows.iter().find(|w| w.id == id)
            .unwrap_or_else(|| abort!("picked id exists")),
        None => {
            synthetic_holder = synthesize_origin(&outputs[0]);
            &synthetic_holder
        }
    };

    let mut band = BandState {
        secondary_low: axes.secondary_low(&origin.rect),
        secondary_high: axes.secondary_high(&origin.rect),
        high_is_top_or_left: false,
    };
    let primary_start_default = axes.primary_forward(&origin.rect);
    let mut primary_start = primary_start_default;
    let mut results: Vec<PassResult> = Vec::new();

    for pass in build_base_passes(flags) {
        apply_base_pass(pass, &mut band, &mut primary_start, primary_start_default,
            origin, outputs, windows, &axes);
        for &endpoint in &[
            EndpointPass::WindowWidth, EndpointPass::OneScreen,
            EndpointPass::TwoScreens, EndpointPass::AllTheWay,
        ] {
            let primary_end = compute_endpoint(endpoint, primary_start, origin, outputs, windows, &axes);
            let hits = cast_ray(windows, &band, primary_start, primary_end, origin.id, &axes);
            if !hits.is_empty() {
                let bbox = hits_bounding_box(&hits)
                    .unwrap_or_else(|| abort!("hits non-empty so bbox exists"));
                let ids = sort_results(hits, flags, dir, origin, &bbox);
                results.push(PassResult { base_pass: pass, endpoint, ids, bbox });
                break;
            }
        }
    }
    results
}

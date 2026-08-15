use smithay::utils::{Logical, Rectangle};
use compositor_support_action_camera_find_direction::Direction;
use compositor_support_action_camera_find_window::WindowEntry;
use compositor_support_action_camera_find_axes::DirAxes;
use compositor_support_action_camera_find_band::best_output_for;
use compositor_support_action_camera_find_passes::EndpointPass;

pub fn compute_endpoint(
    pass: EndpointPass,
    primary_start: f64,
    origin: &WindowEntry,
    outputs: &[Rectangle<f64, Logical>],
    windows: &[WindowEntry],
    axes: &DirAxes,
) -> f64 {
    let origin_len = axes.primary_len(&origin.rect);
    let screen_len = best_output_for(&origin.rect, outputs)
        .map(|o| match axes.dir {
            Direction::Right | Direction::Left => o.size.w,
            Direction::Up | Direction::Down => o.size.h,
            Direction::Diagonal(..) => {
                let (ux, uy) = axes.dir.unit_vec();
                ux.abs() * o.size.w + uy.abs() * o.size.h
            }
        })
        .unwrap_or(0.0);

    match pass {
        EndpointPass::WindowWidth => primary_start + origin_len,
        EndpointPass::OneScreen => primary_start + origin_len + screen_len,
        EndpointPass::TwoScreens => primary_start + origin_len + 2.0 * screen_len,
        EndpointPass::AllTheWay => {
            let max = windows
                .iter()
                .map(|w| axes.primary_forward(&w.rect))
                .fold(f64::NEG_INFINITY, f64::max);
            if max.is_infinite() { primary_start + origin_len } else { max }
        }
    }
}

use smithay::desktop::LayerSurface;
use smithay::utils::{Logical, Point, Size};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::CoordinateTrait;

// The pure placement core lives in the Loop-free interface.core crate; re-export
// so existing `surface_interface_base::position::*` callers are unchanged.
pub use compositor_y5_surface_interface_core::position::*;

/// Rim/draw entry: reads the cursor from the seat (world-space) + the loop's
/// size context, then delegates to the pure core (interface.core). The hit-test
/// path calls the core directly with the point it is already testing.
pub fn layer_surface_position(
    _loop: &Loop,
    layer: &LayerSurface,
    output_size: Size<i32, Logical>,
) -> Point<i32, Logical> {
    let cursor = _loop.state.seat.seat.get_pointer().unwrap().current_location();
    layer_surface_position_core(cursor, _loop.size_ctx_all(), layer, output_size)
}

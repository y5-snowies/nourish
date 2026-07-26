#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod state;
pub mod wire;

pub use state::Orchestrator as DrawState;
pub use state::Orchestrator;
use compositor_support_smithay_dispatch_wire_base::wire::Wire;

pub type Loop = Wire<Orchestrator>;

pub mod export {
    pub use compositor_y5_canvas_state_base::*;
}

pub use compositor_y5_camera_transform_translate::transform::*;
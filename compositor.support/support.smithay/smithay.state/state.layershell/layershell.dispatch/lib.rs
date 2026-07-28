#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod dispatch;
pub mod wire {
    pub use compositor_support_smithay_state_layershell_wire::*;
}
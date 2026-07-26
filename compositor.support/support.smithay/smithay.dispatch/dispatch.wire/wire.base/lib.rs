#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod wire;

// Façade re-exports: keep the old module paths working for any caller.
/// Re-export the old `pub mod delegate` path (callers used ::delegate::).
pub mod delegate {
    pub use super::wire::*;
}
/// Re-export the old `pub mod color_management` path.
pub mod color_management {
    pub use compositor_support_smithay_dispatch_wire_color::color::*;
}

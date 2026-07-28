#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod backend {
    pub use compositor_y5_graphic_display_backend::backend::*;
}
pub mod output;
pub use output::*;

//! Background-shader selection + the loaded-shader Vulkan pass builder, split
//! out of `draw.parallax` so the render element stays within the size policy.

#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod select;
pub use select::*;

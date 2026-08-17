//! Read a live toplevel's `xdg_toplevel_icon_v1` icon (name + decoded buffers).

#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod icon_toplevel;

pub use icon_toplevel::read;

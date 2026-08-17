// Developer logging: bring error!/warn!/info!/trace!/abort! into scope for every module in
// this crate. (Drop this line if the crate genuinely never logs.)
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod drag;

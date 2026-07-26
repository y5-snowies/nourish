// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

// The atomic file write (temp + fsync + rename) and the long-lived worker thread
// that runs it off the frame thread, so a blocking fsync never stalls a frame.
pub mod base;

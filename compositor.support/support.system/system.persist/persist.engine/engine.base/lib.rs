// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

// The persistence engine. `sync` is called at the buffer() mutation boundary for
// the system that just mutated; it compares each of that system's slots to the
// cached last value by `PartialEq` and enqueues an off-thread atomic write only on
// change. The in-memory copy is decoupled from disk — it may run ahead of the file.
pub mod base;

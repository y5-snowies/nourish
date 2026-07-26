// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

// World-start rehydration: load each persisted slot from disk and write the
// reconstructed live value back. Missing files are a normal first run; corrupt
// files are quarantined and defaults kept — never fatal.
pub mod base;

// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

// Load a table's records into its collection slot at world start, optionally
// filtered to one partition (e.g. this world's placeholders). Reconciles the
// table first so corrupt/orphan records and dangling symlinks never load.
pub mod base;

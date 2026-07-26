// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

// Buffer-time table sync: diffs a collection slot's records against a per-table
// cache (by `PartialEq`) and emits per-record put / delete / partition-relink to
// the filesystem store. Writes are synchronous but gated by change, and records
// change rarely (dismiss/edit, or a sampler update that alters the projection).
pub mod base;

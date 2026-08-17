//! Container-runtime lookups (name resolution, running state) via `podman ps`,
//! cached, with a non-blocking path for the extraction hot path.

// Developer logging: brings error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod query;

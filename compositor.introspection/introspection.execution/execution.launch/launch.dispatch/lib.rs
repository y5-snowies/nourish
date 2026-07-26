//! The off-thread launch worker. Mirrors the sampler: a std mpsc carries
//! requests to a dedicated thread; each `LaunchOutcome` is posted back to the
//! calloop loop via a calloop channel `Sender`, so the general `Executed`
//! dispatch runs on the compositor thread.

// Developer logging: error!/warn!/info!/trace!/abort! in scope for this crate.
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod dispatch;

//! Child hygiene: what a spawned process must NOT inherit from the compositor —
//! the reaper's blocked signals, our file descriptors, our CPU-priority boost.
//! Runs in the forked child before `exec`; EVERY `Command` the compositor
//! spawns installs it as the `pre_exec` hook, app launches and short-lived
//! helpers alike.

// Developer logging: error!/warn!/info!/trace!/abort! in scope for this crate.
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod hygiene;

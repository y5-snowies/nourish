//! Loader-side wiring for the launch executor: `install` builds the Executor, stores it
//! as driver data, registers its calloop sources (the outcome receiver and the reaper),
//! and names the destination every detached spawn in the process sends its child's exit
//! descriptor to. Keeps the loader's `main` free of execution-service detail.
//!
//! There is no startup signal step any more. It used to also export a `block_sigchld`
//! for `main` to call before spawning threads, because the reaper was a SIGCHLD
//! `signalfd` and a signalfd only receives what is blocked. Children are collected
//! through their own exit descriptors now, so nothing consumes that signal.

// Developer logging: error!/warn!/abort! in scope for this crate.
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod install;

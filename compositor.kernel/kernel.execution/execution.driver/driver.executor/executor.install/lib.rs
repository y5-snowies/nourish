//! Loader-side wiring for the launch executor: `block_sigchld` (call at the top
//! of main, before threads) and `install` (build the Executor, store it as
//! driver data, and register its calloop sources — outcome receiver + reaper).
//! Keeps the loader's `main` free of execution-service detail.

// Developer logging: error!/warn!/abort! in scope for this crate.
#[macro_use]
extern crate compositor_model_debug_instance_record;

pub mod install;

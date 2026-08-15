//! compositor.developer structured logging — **backend runtime** (façade).
//!
//! One drain thread consumes the global fan-in buffer (printing, history,
//! broadcast — `process.instance.drain`); a second thread runs the tonic
//! server-streaming `LogStream` service on a unix socket (`process.instance.serve`).
//! The generated proto bindings + socket path live in `process.instance.bind` and are
//! re-exported here. Started once by `compositor_model_log_process_main::spawn`.

pub mod instance;
pub use instance::*;

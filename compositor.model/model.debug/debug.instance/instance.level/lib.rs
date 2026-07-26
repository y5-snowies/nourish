//! Logging frontend data types: [`Level`], [`Instance`], [`Record`], and the
//! `COMPOSITOR_LOG_LEVEL` parser. Split out of `instance.record` (the macro crate).

use std::time::Instant;

/// Log levels, most → least severe. The discriminant doubles as the mask bit index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Trace = 3,
}

impl Level {
    /// Fixed-width label for aligned dmesg-style output.
    pub const fn label(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
            Level::Trace => "TRACE",
        }
    }

    const fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// Per-crate static instance. Holds the full crate name as `&'static str` (no alloc).
pub struct Instance {
    pub crate_name: &'static str,
}

impl Instance {
    pub const fn new(crate_name: &'static str) -> Self {
        Self { crate_name }
    }
}

/// One structured log record. `message` is the only per-record allocation; the crate and
/// function tags are `&'static str`.
pub struct Record {
    pub level: Level,
    pub crate_name: &'static str,
    pub function: &'static str,
    pub message: String,
    pub at: Instant,
    /// When set, the drain signals this channel after fully handling the record (printed +
    /// streamed). Used by `abort!` to block until its message has been emitted before the
    /// process panics. `None` for normal logs.
    pub ack: Option<crossbeam_channel::Sender<()>>,
}

impl Record {
    /// Build a record from a crate-name string directly (what the macros use —
    /// `env!("CARGO_PKG_NAME")`, a `&'static str`, so no per-crate `instance!()` is needed).
    pub fn with(
        level: Level,
        crate_name: &'static str,
        function: &'static str,
        message: String,
    ) -> Self {
        Self { level, crate_name, function, message, at: Instant::now(), ack: None }
    }

    /// Build a record from an [`Instance`] (kept for back-compat with `instance!()`).
    pub fn new(level: Level, instance: &Instance, function: &'static str, message: String) -> Self {
        Self::with(level, instance.crate_name, function, message)
    }
}

/// Parse a `COMPOSITOR_LOG_LEVEL` value like `"info,trace,error,warn"` into a mask.
pub fn parse_levels(spec: &str) -> u8 {
    let mut mask = 0u8;
    for part in spec.split(',') {
        match part.trim().to_ascii_lowercase().as_str() {
            "error" => mask |= Level::Error.bit(),
            "warn" => mask |= Level::Warn.bit(),
            "info" => mask |= Level::Info.bit(),
            "trace" => mask |= Level::Trace.bit(),
            _ => {}
        }
    }
    mask
}

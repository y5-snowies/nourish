//! Minimal interactive stdin prompts (no external TUI dependency).
//!
//! Every prompt shows a `[default]`; an empty line keeps it. When stdin is not a
//! TTY (or is closed), prompts fall back to the default so the installer can run
//! non-interactively / piped.

pub mod prompt;
pub use prompt::*;

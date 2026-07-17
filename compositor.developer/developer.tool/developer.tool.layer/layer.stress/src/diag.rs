//! Plain-stderr diagnostics. This tool lives outside the compositor's structured-logging
//! graph (see `dev-tools-placement`), so it just prints prefixed lines to stderr. Each
//! process tags its lines with a short role prefix so controller/subject output interleaves
//! readably in one terminal.

use std::sync::OnceLock;

static ROLE: OnceLock<&'static str> = OnceLock::new();

/// Set the short role tag (`"controller"` / `"subject"`) shown on every line. Call once.
pub fn set_role(role: &'static str) {
    let _ = ROLE.set(role);
}

fn role() -> &'static str {
    ROLE.get().copied().unwrap_or("harness")
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{ $crate::diag::emit("INFO", format!($($arg)*)); }};
}
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{ $crate::diag::emit("WARN", format!($($arg)*)); }};
}
#[macro_export]
macro_rules! err {
    ($($arg:tt)*) => {{ $crate::diag::emit("ERR ", format!($($arg)*)); }};
}

/// Backing function for the `info!`/`warn!`/`err!` macros.
pub fn emit(level: &str, msg: String) {
    eprintln!("[{} {}] {}", role(), level, msg);
}

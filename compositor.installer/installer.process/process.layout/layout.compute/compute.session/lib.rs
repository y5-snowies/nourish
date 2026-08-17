//! Per-preset session file text generators (wrapper script, systemd service,
//! shutdown target, wayland-session entry), modeled on the live reference captures
//! kept under .reference/references/ (live-installation-example-files/ and installer-artifact/).

pub mod session;
pub use session::*;

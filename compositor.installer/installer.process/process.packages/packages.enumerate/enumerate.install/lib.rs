//! The system package-install runner, generic over the detected package manager
//! (`dnf` / `apt-get` / `pacman`), plus the Debian `bookworm-backports` enabler. The
//! Fedora-only RPM Fusion steps live in the sibling `enumerate.rpmfusion` crate (which
//! reuses `run_sudo` from here). Pure std.

pub mod install;
pub use install::*;

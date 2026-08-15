//! Distro / package-manager detection from `/etc/os-release`.
//!
//! Every multiarch bundle ships the same `y5-install`, so the installer must learn at
//! runtime which package manager to drive. We classify the running distro by its
//! `os-release` `ID`/`ID_LIKE` into one of three manager families and expose the
//! `VERSION_ID` (used only to resolve the few version-suffixed apt package names).
//! Pure std.

pub mod platform;
pub use platform::*;

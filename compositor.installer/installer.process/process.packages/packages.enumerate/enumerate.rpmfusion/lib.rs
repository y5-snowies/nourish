//! Fedora-only RPM Fusion enablement + the full-FFmpeg swap. Kept apart from the generic
//! `enumerate.install` runner so the Fedora-isms don't leak into the apt/pacman paths.
//! Only ever called on the `Dnf` path (execute.packages). Pure std.

pub mod rpmfusion;
pub use rpmfusion::*;

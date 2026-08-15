//! Package-group dispatcher: pick the per-distro runtime table for the detected package
//! manager. The group *structure* (keys `runtime`/`xwayland`/`devtool`/`diagnostics`/
//! `toolchain`, titles, defaults) is identical across distros — only the package **names**
//! differ, so each manager gets its own sibling table crate (`enumerate.fedora` /
//! `enumerate.debian` / `enumerate.arch` / `enumerate.nixos`). Pure std.

pub mod groups;
pub use groups::*;

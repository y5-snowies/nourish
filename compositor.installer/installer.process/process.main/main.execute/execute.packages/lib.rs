//! Installer step 1: detect the distro's package manager + GPU, prompt the package
//! groups, install the selection with that manager, and wire up hardware VA-API per
//! platform. NixOS is special-cased: it prints a `configuration.nix` (nix-ld) snippet to
//! add instead of installing. On NVIDIA we only check the bound driver and warn (Nourish
//! never installs the proprietary NVIDIA driver).

pub mod packages;
pub use packages::*;

//! Package enumeration façade: distro/manager detection, the per-distro package groups,
//! GPU-vendor detection, the generic install runner, the NixOS profile renderer, and the
//! Fedora-only RPM Fusion helpers. The implementation lives in the sibling `enumerate.*`
//! crates; this crate re-exports their public surface. Pure std.

pub use compositor_installer_process_packages_enumerate_platform::{PackageManager, release_id};
pub use compositor_installer_process_packages_enumerate_groups::groups;
pub use compositor_installer_process_packages_enumerate_install::{
    apt_enable_backports, pkg_install, run_sudo,
};
pub use compositor_installer_process_packages_enumerate_rpmfusion::{
    enable_rpmfusion_free, enable_rpmfusion_nonfree, swap_ffmpeg_full,
};
pub use compositor_installer_process_packages_enumerate_nixos::{render_flake, render_profile};
pub use compositor_installer_process_packages_enumerate_model::{
    Gpu, NvidiaDriver, PackageGroup, capture_encoder_for, detect_gpu, nvidia_driver_status,
};

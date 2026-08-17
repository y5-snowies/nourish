//! Fedora (`dnf`) runtime package groups — the original, canonical table.
//!
//! The `runtime` group is the authoritative *runtime* dependency set for the prebuilt
//! `y5_compositor` binary, derived from the binary itself: directly linked sonames from
//! its ELF `NEEDED` entries (`readelf -d`), plus the `dlopen`-loaded sonames (Wayland,
//! Vulkan, EGL). The end user needs only these shared libraries plus a GPU driver — NOT
//! the Rust toolchain or `-devel` headers (the opt-in `toolchain` group). See the
//! sibling `enumerate.debian` / `enumerate.arch` / `enumerate.nixos` tables for the same
//! set expressed in each other package manager's names.
//!
//! soname -> Fedora runtime package (the mapping encoded below):
//!   libpam.so.0         -> pam                 libdbus-1.so.3   -> dbus-libs
//!   libpulse.so.0       -> pulseaudio-libs     libudev.so.1     -> systemd-libs
//!   libgbm.so.1         -> mesa-libgbm         libseat.so.1     -> libseat
//!   libinput.so.10      -> libinput            libxkbcommon.so.0-> libxkbcommon
//!   libpixman-1.so.0    -> pixman
//!   libwayland-{client,server,egl}.so* -> libwayland-{client,server} + libwayland-egl
//!   libvulkan.so.1      -> vulkan-loader (+ mesa-vulkan-drivers / the NVIDIA ICD)
//!   libEGL.so.1         -> libglvnd-egl (+ mesa-libEGL / NVIDIA)
//!
//! NVIDIA note: Nourish does NOT install the proprietary NVIDIA driver (akmod build +
//! reboot + Secure Boot signing — the user's call); the installer only checks the bound
//! driver and warns. Pure std.

pub mod fedora;
pub use fedora::*;

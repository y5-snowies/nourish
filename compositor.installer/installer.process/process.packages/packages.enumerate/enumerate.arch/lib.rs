//! Arch Linux (`pacman`) runtime package groups — the same soname set as the Fedora
//! table (`enumerate.fedora`) in Arch names. Arch is rolling, so `release` is unused and
//! there are no soversion-suffixed names. Pure std.
//!
//! Arch bundles many sonames into one package: `mesa` provides libgbm/libEGL/libGL/DRI,
//! `wayland` provides the client/server/egl libs, `seatd` provides libseat. The one gap
//! is Vulkan: Arch has NO generic `mesa-vulkan-drivers` — the ICD is vendor-split
//! (`vulkan-radeon` / `vulkan-intel` / …). The runtime group therefore ships the loader +
//! the software driver (`vulkan-swrast`, always works); execute.packages adds the
//! vendor ICD for the detected GPU (mirroring the VA-API driver choice).

pub mod arch;
pub use arch::*;

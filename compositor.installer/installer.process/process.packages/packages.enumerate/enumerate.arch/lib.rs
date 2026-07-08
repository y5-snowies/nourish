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

use compositor_installer_process_packages_enumerate_model::PackageGroup;

/// Pacman package groups. `_release` is unused (Arch is rolling; names carry no soversion).
pub fn groups(_release: Option<&str>) -> Vec<PackageGroup> {
    vec![
        PackageGroup {
            key: "runtime",
            title: "y5 runtime libraries (required)",
            description: "Exact shared libs the prebuilt compositor links/dlopens: \
                          Wayland, input/seat/udev, GBM/DRM, pixman, Vulkan/EGL loader \
                          + software Mesa driver, PAM, dbus, PulseAudio, FFmpeg",
            packages: vec![
                // Directly linked (many sonames per Arch package).
                "pam", "dbus", "libpulse", "systemd-libs",
                "libinput", "seatd", "libxkbcommon", "pixman",
                "mesa", "libdrm", "libdisplay-info",
                "ffmpeg",
                // dlopen'd Wayland libs (all in `wayland`).
                "wayland",
                // dlopen'd render stack: loader + software ICD + glvnd dispatch. The
                // vendor Vulkan ICD (vulkan-radeon/vulkan-intel) is added per-GPU in
                // execute.packages; `mesa` already carries the EGL/GL vendor libs.
                "vulkan-icd-loader", "vulkan-swrast", "libglvnd",
            ],
            default_on: true,
        },
        PackageGroup {
            key: "xwayland",
            title: "XWayland / X11 compatibility",
            description: "Run X11 clients under the compositor (runtime only)",
            packages: vec!["xorg-xwayland"],
            default_on: true,
        },
        PackageGroup {
            key: "devtool",
            title: "Developer tool window (log viewer)",
            description: "WebKitGTK / GTK runtime libs for the prebuilt dev window",
            // webkit2gtk-4.1 pulls gtk3 + libsoup3 as dependencies.
            packages: vec!["webkit2gtk-4.1", "librsvg", "libayatana-appindicator"],
            default_on: true,
        },
        PackageGroup {
            key: "diagnostics",
            title: "Diagnostics & terminals (optional)",
            description: "vulkan/egl/gl info tools and a terminal",
            packages: vec!["vulkan-tools", "mesa-utils", "foot", "wev"],
            default_on: false,
        },
        PackageGroup {
            key: "toolchain",
            title: "Build-from-source toolchain (NOT needed for the prebuilt install)",
            description: "Rust/cargo, clang, protobuf and the build libs — only if you \
                          intend to compile y5 on this machine",
            packages: vec![
                "base-devel", "git", "clang", "pkgconf", "protobuf", "rust",
                "wayland-protocols", "libxcb", "xcb-util-cursor",
            ],
            default_on: false,
        },
    ]
}

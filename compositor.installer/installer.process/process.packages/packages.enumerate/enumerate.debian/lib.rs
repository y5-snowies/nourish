//! Debian / Ubuntu (`apt`) runtime package groups — the same soname set as the Fedora
//! table (`enumerate.fedora`) expressed in apt names. Pure std.
//!
//! Two cross-release hazards are handled here rather than left to fail:
//!   * ffmpeg: the libav* runtime libs are soversion-suffixed and differ per release
//!     (bookworm libavcodec59, noble …60, trixie …61). We install the `ffmpeg` package
//!     instead — it depends on exactly the matching libav* runtime, so the right
//!     soversion is pulled without us naming it. Release-independent.
//!   * libdisplay-info: soversion-suffixed with NO metapackage, so it IS named per
//!     release — `libdisplay-info2` on bookworm(-backports)/trixie, `libdisplay-info1`
//!     on Ubuntu noble. (bookworm carries it only in bookworm-backports, so the apt path
//!     enables that suite on release 12 — see execute.packages / enumerate.install.)
//!
//! GTK is deliberately NOT named: the `64-bit time_t` transition renamed it to
//! `libgtk-3-0t64` on trixie + noble but left it `libgtk-3-0` on bookworm. Installing
//! `libwebkit2gtk-4.1-0` pulls the correct GTK for the release as a dependency, so the
//! devtool group sidesteps the t64 rename entirely.

use compositor_installer_process_packages_enumerate_model::PackageGroup;

/// Apt package groups. `release` is the `VERSION_ID` (`"12"`, `"13"`, `"24.04"`) used to
/// pick the one soversion-suffixed name (`libdisplay-info`).
pub fn groups(release: Option<&str>) -> Vec<PackageGroup> {
    vec![
        PackageGroup {
            key: "runtime",
            title: "y5 runtime libraries (required)",
            description: "Exact shared libs the prebuilt compositor links/dlopens: \
                          Wayland, input/seat/udev, GBM/DRM, pixman, Vulkan/EGL loader \
                          + generic Mesa driver, PAM, dbus, PulseAudio, FFmpeg",
            packages: vec![
                // Directly linked (ELF NEEDED).
                "libpam0g", "libdbus-1-3", "libpulse0", "libudev1",
                "libinput10", "libseat1", "libxkbcommon0", "libpixman-1-0",
                "libgbm1", "libdrm2", display_info(release),
                // FFmpeg runtime libs — the `ffmpeg` package pulls the exact libav*
                // runtime for this release (avoids naming the soversion).
                "ffmpeg",
                // dlopen'd Wayland libs.
                "libwayland-client0", "libwayland-server0", "libwayland-egl1",
                // dlopen'd render stack: loaders + dispatch + generic Mesa driver.
                "libvulkan1", "mesa-vulkan-drivers",
                "libglvnd0", "libegl1", "libgles2", "libgl1",
                "libegl-mesa0", "libgl1-mesa-dri",
            ],
            default_on: true,
        },
        PackageGroup {
            key: "xwayland",
            title: "XWayland / X11 compatibility",
            description: "Run X11 clients under the compositor (runtime only)",
            packages: vec!["xwayland"],
            default_on: true,
        },
        PackageGroup {
            key: "devtool",
            title: "Developer tool window (log viewer)",
            description: "WebKitGTK / GTK runtime libs for the prebuilt dev window",
            // libwebkit2gtk-4.1-0 pulls GTK3 (the correct libgtk-3-0 / -0t64 for the
            // release) + libsoup3 as dependencies, so they aren't named explicitly.
            packages: vec![
                "libwebkit2gtk-4.1-0", "librsvg2-2", "libayatana-appindicator3-1",
            ],
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
            description: "clang, protobuf, and every -dev header — only if you intend to \
                          compile y5 on this machine (rust still comes from rustup)",
            packages: vec![
                "build-essential", "clang", "libclang-dev", "pkg-config", "git",
                "curl", "ca-certificates", "protobuf-compiler", "libprotobuf-dev",
                "libpam0g-dev", "libinput-dev", "libseat-dev", "libxkbcommon-dev",
                "libpixman-1-dev", "libsystemd-dev", "libudev-dev", "libwayland-dev",
                "wayland-protocols", "libegl-dev", "libgles-dev", "libgl-dev",
                "libgbm-dev", "libglvnd-dev", "libvulkan-dev", "libdrm-dev",
                "libavcodec-dev", "libavformat-dev", "libavutil-dev", "libavfilter-dev",
                "libavdevice-dev", "libswscale-dev", "libswresample-dev",
                "libdbus-1-dev", "libpulse-dev", "libdisplay-info-dev",
                "libwebkit2gtk-4.1-dev", "libsoup-3.0-dev", "libgtk-3-dev",
                "librsvg2-dev", "libayatana-appindicator3-dev",
                "libxcb1-dev", "libxcb-cursor-dev", "patchelf",
            ],
            default_on: false,
        },
    ]
}

/// The soversion-suffixed EDID library, per release. The soversion tracks each release's
/// `libdisplay-info-dev` (what the bundle was built against), NOT just "the newest":
///   * Ubuntu 26.04 (resolute)          → 0.3.0 → `libdisplay-info3`
///   * Debian 12 (backports) / 13       → 0.2.0 → `libdisplay-info2`
///   * Ubuntu 24.04 (noble) + fallback  → 0.1.1 → `libdisplay-info1`
/// NOTE: several soversions can coexist in one release's repos (e.g. trixie carries both
/// `2` and `3`), so a name-availability check alone won't catch a wrong pick here — these
/// are pinned from each release's `libdisplay-info-dev` dependency.
fn display_info(release: Option<&str>) -> &'static str {
    match release {
        Some("26.04") => "libdisplay-info3",
        Some("12") | Some("13") => "libdisplay-info2",
        _ => "libdisplay-info1",
    }
}

/// True when this apt release needs `bookworm-backports` enabled to reach
/// `libdisplay-info2` (Debian 12 only; trixie/noble carry it in main/universe). The
/// install path calls this to add the suite before the transaction. See execute.packages.
pub fn needs_backports(release: Option<&str>) -> bool {
    release == Some("12")
}

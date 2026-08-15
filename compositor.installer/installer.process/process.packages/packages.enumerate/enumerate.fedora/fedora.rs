use compositor_installer_process_packages_enumerate_model::PackageGroup;

/// Fedora package groups. `_release` is unused (dnf package names don't carry the release
/// soversion the way Debian's do); the parameter keeps a uniform signature across tables.
///
/// Each entry installs ONLY packages base Fedora repos carry, so a strict (abort-on-missing)
/// `dnf install` over any selection succeeds; RPM-Fusion-only bits (hardware VA-API) are a
/// separate opt-in handled in execute.packages.
pub fn groups(_release: Option<&str>) -> Vec<PackageGroup> {
    vec![
        PackageGroup {
            key: "runtime",
            title: "y5 runtime libraries (required)",
            description: "Exact shared libs the prebuilt compositor links/dlopens: \
                          Wayland, input/seat/udev, GBM/DRM, pixman, Vulkan/EGL loader \
                          + generic Mesa driver, PAM, dbus, PulseAudio, FFmpeg",
            packages: vec![
                // Directly linked (ELF NEEDED).
                "pam", "dbus-libs", "pulseaudio-libs", "systemd-libs",
                "libinput", "libseat", "libxkbcommon", "pixman",
                "mesa-libgbm", "libdrm", "libdisplay-info",
                // FFmpeg 8.x runtime libs — screen capture / video encode, linked by
                // capture.vaapi (Fedora ships these as the libre `-free` build).
                "libavutil-free", "libavcodec-free", "libavformat-free",
                "libavfilter-free", "libswscale-free",
                // dlopen'd Wayland libs.
                "libwayland-client", "libwayland-server", "libwayland-egl",
                // dlopen'd render stack: loaders + dispatch + generic Mesa driver
                // (the vendor-specific driver comes from the matching group below).
                "vulkan-loader", "mesa-vulkan-drivers",
                "libglvnd-egl", "libglvnd-gles", "libglvnd-opengl",
                "mesa-libEGL", "mesa-libGL", "mesa-dri-drivers",
            ],
            default_on: true,
        },
        PackageGroup {
            key: "xwayland",
            title: "XWayland / X11 compatibility",
            description: "Run X11 clients under the compositor (runtime only)",
            packages: vec!["xorg-x11-server-Xwayland"],
            default_on: true,
        },
        PackageGroup {
            key: "devtool",
            title: "Developer tool window (log viewer)",
            description: "WebKitGTK / GTK runtime libs for the prebuilt dev window",
            packages: vec![
                "webkit2gtk4.1", "libsoup3", "gtk3",
                "librsvg2", "libappindicator-gtk3",
            ],
            default_on: true,
        },
        PackageGroup {
            key: "diagnostics",
            title: "Diagnostics & terminals (optional)",
            description: "vulkan/egl/glx info tools, glmark2, a couple of terminals",
            packages: vec![
                "vulkan-tools", "egl-utils", "glx-utils", "mesa-demos", "glmark2",
                "foot", "alacritty", "wev",
            ],
            default_on: false,
        },
        PackageGroup {
            key: "toolchain",
            title: "Build-from-source toolchain (NOT needed for the prebuilt install)",
            description: "Rust/cargo, clang, protobuf, and every -devel header — only if \
                          you intend to compile y5 on this machine",
            packages: vec![
                "cargo", "rust", "git", "clang-devel", "pkgconf-pkg-config", "mold",
                "pam-devel", "libdisplay-info-devel", "libinput-devel", "libseat-devel",
                "libxkbcommon-devel", "pixman-devel", "systemd-devel",
                "wayland-devel", "wayland-protocols-devel", "mesa-libgbm-devel",
                "vulkan-loader-devel", "mesa-libEGL-devel", "mesa-libGL-devel",
                "libglvnd-devel", "libX11-devel", "libxcb-devel", "xcb-util-cursor-devel",
                "protobuf", "protobuf-devel", "protobuf-compiler",
                "dbus-devel", "pulseaudio-libs-devel", "openssl-devel",
                "ffmpeg-free-devel",
                "webkit2gtk4.1-devel", "libsoup3-devel", "gtk3-devel",
                "librsvg2-devel", "libappindicator-gtk3-devel", "patchelf",
            ],
            default_on: false,
        },
    ]
}

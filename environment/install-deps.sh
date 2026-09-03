#!/usr/bin/env bash
# Install the host build dependencies for a bare-metal (non-container) build on Fedora.
# Usage: ./install-deps.sh
set -e

# Wayland / smithay core devel
sudo dnf install -y \
    libinput-devel libseat-devel libxkbcommon-devel \
    pixman-devel clang-devel wayland-devel \
    wayland-protocols-devel mesa-libgbm-devel \
    libdisplay-info-devel systemd-devel \
    dbus-devel pam-devel

# Build tools (protoc for prost-build)
sudo dnf install -y \
    protobuf-compiler


# Graphics stack (Intel/VA-API for bare-metal) + ffmpeg 8.x (screen capture / video encode)
sudo dnf install -y \
    mesa-libEGL-devel \
    mesa-libGL-devel \
    mesa-libgbm-devel \
    libglvnd-devel \
    ffmpeg-free-devel \
    pulseaudio-libs-devel

# Compiler toolchain (default system linker; mold no longer used)
sudo dnf install -y clang

# Diagnostics
sudo dnf install -y egl-utils mesa-demos

# Install-bundle components (match ci/Containerfile):
# Developer-tool window (Tauri 2) GUI devel
sudo dnf install -y \
    webkit2gtk4.1-devel libsoup3-devel gtk3-devel \
    librsvg2-devel libappindicator-gtk3-devel

# X11 — smithay's `xwayland` feature builds the in-process X11 window manager against
# x11rb, which is pure Rust and links no C library. Only the Xwayland SERVER itself has
# to be installed, to run X11 clients at all.
sudo dnf install -y \
    xorg-x11-server-Xwayland


# --- Workspace tooling ------------------------------------------------------
# cargo-shear backs the `deps-unused` lint rule: it parses each crate's Rust with
# syn and reports dependencies nothing references. The rule is skipped (with a
# note) when the binary is absent, so this is a convenience, not a build
# requirement — but a crate.json that accumulates unused entries will not be
# caught locally without it.
#
# --locked so the tool itself is reproducible.
cargo install --locked cargo-shear

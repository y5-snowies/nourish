#!/usr/bin/env bash
# Install the host build dependencies for a bare-metal (non-container) build on Fedora.
# Usage: ./install-deps.sh
# Abort on any error, unset variable, or failure anywhere in a pipeline (e.g. the
# `curl ... | sh` rustup bootstrap below — plain `set -e` only checks the last
# command in a pipe, so a curl failure would otherwise be masked).
set -euo pipefail

# Wayland / smithay core devel
sudo dnf install -y \
    libinput-devel libseat-devel libxkbcommon-devel \
    pixman-devel clang-devel wayland-devel \
    wayland-protocols-devel mesa-libgbm-devel \
    libdisplay-info-devel systemd-devel \
    dbus-devel pam-devel

# Build tools (protoc for prost-build; curl for the rustup bootstrap below)
sudo dnf install -y \
    protobuf-compiler curl


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

# Rust toolchain (cargo/rustc). The repo pins the channel via rust-toolchain.toml
# ("stable"), which rustup honours automatically on first build. Install rustup if
# cargo isn't already on PATH; otherwise leave the existing install untouched.
if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # Make cargo available in the current shell for the rest of this script/session.
    . "$HOME/.cargo/env"
fi

# Diagnostics
sudo dnf install -y egl-utils mesa-demos

# Install-bundle components (match ci/Containerfile):
# Developer-tool window (Tauri 2) GUI devel
sudo dnf install -y \
    webkit2gtk4.1-devel libsoup3-devel gtk3-devel \
    librsvg2-devel libappindicator-gtk3-devel

# xwayland-satellite (X11/XCB)
sudo dnf install -y \
    libxcb-devel xcb-util-cursor-devel


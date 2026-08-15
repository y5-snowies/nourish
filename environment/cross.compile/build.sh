#!/usr/bin/env bash
# Cross-compile y5_compositor for aarch64 on an x86_64 host, against the local
# sysroot in this directory. No container, no access to the target device.
#
# Usage: ./build.sh [winit|udev|native] [release-fast|release]
#
# It only builds — copying the result to the device is yours to do. Remember to
# `setcap cap_sys_nice+ep` it there: the capability cannot be set on a foreign-arch
# binary here, and a copy would drop it anyway.
#
# One-time host setup:
#   sudo dnf install -y gcc-aarch64-linux-gnu binutils-aarch64-linux-gnu rustup
#   rustup-init -y --no-modify-path --profile minimal
#   rustup target add aarch64-unknown-linux-gnu
#   ./sysroot.sh 44                 # populate ./fedora44 (once)
#
# Env:
#   Y5_SYSROOT      sysroot to link against (default: <this dir>/fedora44)
#   Y5_TARGET_DIR   cargo target dir (default ~/.cache/y5-cross/target — deliberately NOT
#                   the host target/, so host and cross builds don't invalidate each
#                   other's build-script and proc-macro artifacts on every switch)
#   Y5_CROSS_CPU    -C target-cpu, e.g. cortex-a76 (Pi 5) / cortex-a72 (Pi 4).
#                   Unset = generic aarch64, which runs on any of them.
#
# This is a SIBLING of environment/build.sh, not a wrapper around it: cross builds
# put the binary under an extra <triple>/ level in the target dir, which that script
# does not model. The backend/profile mapping below is kept identical to it.
#
# Note: rustflags live in .cargo/config.toml. Do NOT set RUSTFLAGS here — it replaces
# that config wholesale (dropping -A warnings). The per-target env var used below is
# the one channel cargo JOINS with the config instead of replacing.
set -euo pipefail

BACKEND="${1:-udev}"    # cross builds target real hardware, so udev is the useful default
PROFILE="${2:-fast}"    # release-fast: full optimization, no LTO — skips the serial link

TRIPLE=aarch64-unknown-linux-gnu
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SYSROOT="${Y5_SYSROOT:-$HERE/fedora44}"
TARGET_DIR="${Y5_TARGET_DIR:-$HOME/.cache/y5-cross/target}"

# --- Preconditions ---------------------------------------------------------
command -v aarch64-linux-gnu-gcc >/dev/null 2>&1 \
    || { echo "build.sh: aarch64-linux-gnu-gcc missing — sudo dnf install gcc-aarch64-linux-gnu binutils-aarch64-linux-gnu" >&2; exit 1; }
[ -e "$SYSROOT/usr/lib64/libc.so" ] \
    || { echo "build.sh: no usable sysroot at $SYSROOT — run ./sysroot.sh first" >&2; exit 1; }

# rustup's cargo, not Fedora's /usr/bin/cargo: Fedora packages rust-std only for
# wasm/uefi/none targets, so the system toolchain cannot build for aarch64 linux.
export PATH="$HOME/.cargo/bin:$PATH"
command -v rustup >/dev/null 2>&1 \
    || { echo "build.sh: rustup not found — sudo dnf install rustup && rustup-init -y --no-modify-path --profile minimal" >&2; exit 1; }
rustup target list --installed 2>/dev/null | grep -qx "$TRIPLE" \
    || { echo "build.sh: missing std for $TRIPLE — run: rustup target add $TRIPLE" >&2; exit 1; }

# --- Repo root + entry crate (same discovery rules as environment/build.sh) --
REPO_ROOT="${Y5_REPO_ROOT:-}"
if [ -z "$REPO_ROOT" ]; then
    d="$HERE"
    while [ "$d" != "/" ]; do
        if compgen -G "$d/compositor*/Cargo.toml" >/dev/null 2>&1; then REPO_ROOT="$d"; break; fi
        d="$(dirname "$d")"
    done
fi
[ -n "$REPO_ROOT" ] || { echo "build.sh: could not locate repo root (no compositor* dir found)" >&2; exit 1; }

EXECUTE_DIR="$(dirname "$(grep -rl --include=Cargo.toml --exclude-dir=target --exclude-dir=node_modules 'name *= *"y5_compositor"' "$REPO_ROOT"/compositor* | head -n1)")"
[ -n "$EXECUTE_DIR" ] && [ -d "$EXECUTE_DIR" ] || { echo "build.sh: could not find the y5_compositor crate" >&2; exit 1; }

# --- Backend -> cargo feature / profile -> subdir ---------------------------
feature_args=()
case "$BACKEND" in
    winit) ;;
    udev|native) feature_args=(--no-default-features --features backend-native) ;;
    *) echo "build.sh: unknown backend '$BACKEND' (expected winit|udev|native)" >&2; exit 1 ;;
esac
# Mirrors the host build.sh: no `debug`. `release` here is the fat-LTO profile
# (the cross equivalent of build-optimized.sh), and Y5_PROFILE selects it too so
# both scripts answer to the same env var.
PROFILE="${Y5_PROFILE:-$PROFILE}"
case "$PROFILE" in
    fast|release-fast) profile_args=(--profile release-fast) ; sub=release-fast ;;
    release) profile_args=(--release) ; sub=release ;;
    *) echo "build.sh: unknown profile '$PROFILE' (expected release-fast|release)" >&2; exit 1 ;;
esac

# --- Linker / C compiler shim ----------------------------------------------
# rustc gives no way to inject --sysroot per link invocation and cc-rs needs the
# same view of the world, so both go through one generated wrapper.
# -rpath-link lets ld resolve the sysroot's own inter-library dependencies
# (libEGL -> libGLdispatch, libavformat -> libavcodec, ...) without reaching for
# the host's x86_64 copies.
libdirs=("$SYSROOT/usr/lib64" "$SYSROOT/usr/lib")
CC_WRAP="$HERE/.shim/${TRIPLE}-cc"
mkdir -p "$HERE/.shim"
{
    echo '#!/bin/sh'
    echo '# generated by environment/cross.compile/build.sh — do not edit'
    printf 'exec aarch64-linux-gnu-gcc --sysroot="%s"' "$SYSROOT"
    for d in "${libdirs[@]}"; do printf ' -L"%s" -Wl,-rpath-link,"%s"' "$d" "$d"; done
    echo ' "$@"'
} > "$CC_WRAP"
chmod +x "$CC_WRAP"

# --- Cross environment ------------------------------------------------------
export CARGO_BUILD_TARGET="$TRIPLE"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$CC_WRAP"
[ -n "${Y5_CROSS_CPU:-}" ] && export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C target-cpu=${Y5_CROSS_CPU}"

# cc-rs: target compiler = the shim, host compiler stays native (build scripts and
# proc macros are compiled for and RUN on this machine).
export CC_aarch64_unknown_linux_gnu="$CC_WRAP"
export CXX_aarch64_unknown_linux_gnu="aarch64-linux-gnu-g++"
export AR_aarch64_unknown_linux_gnu="aarch64-linux-gnu-ar"
export HOST_CC="${HOST_CC:-cc}"
export HOST_CXX="${HOST_CXX:-c++}"

# pkg-config: read .pc files from the sysroot and prefix every -I/-L with it.
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_SYSROOT_DIR="$SYSROOT"
pcpath="$(find "$SYSROOT/usr/lib64/pkgconfig" "$SYSROOT/usr/share/pkgconfig" -maxdepth 0 -type d -printf '%p:' 2>/dev/null)"
# Must be non-empty: pkg-config reads an EMPTY PKG_CONFIG_LIBDIR as "use the built-in
# defaults", i.e. the HOST's x86_64 .pc files — which then yield host paths with the
# sysroot glued on the front and fail much later as missing headers.
[ -n "$pcpath" ] || { echo "build.sh: no pkgconfig dirs under $SYSROOT — re-run ./sysroot.sh" >&2; exit 1; }
export PKG_CONFIG_LIBDIR="${pcpath%:}"

# bindgen (drm-sys, gbm-sys, input-sys, pixman-sys, pam-sys, ...) must read the
# sysroot's headers rather than /usr/include.
export BINDGEN_EXTRA_CLANG_ARGS="--sysroot=$SYSROOT -I$SYSROOT/usr/include ${BINDGEN_EXTRA_CLANG_ARGS:-}"

echo ">> cross-building y5_compositor [backend=$BACKEND profile=$PROFILE target=$TRIPLE]" >&2
echo ">> sysroot: $SYSROOT" >&2
( cd "$EXECUTE_DIR" && cargo build "${profile_args[@]}" "${feature_args[@]}" --target-dir="$TARGET_DIR" >&2 )

BIN="$TARGET_DIR/$TRIPLE/$sub/y5_compositor"
[ -f "$BIN" ] || { echo "build.sh: expected binary not found at $BIN" >&2; exit 1; }
chmod +x "$BIN"

# A misconfigured sysroot can still produce a host-arch binary; catch that here
# rather than on the device.
machine="$(readelf -h "$BIN" | awk -F: '/Machine:/ {gsub(/^ +/,"",$2); print $2}')"
[ "$machine" = "AArch64" ] || { echo "build.sh: built binary is '$machine', not AArch64" >&2; exit 1; }
echo ">> ok: AArch64 binary, $(du -h "$BIN" | cut -f1)" >&2

echo "$BIN"

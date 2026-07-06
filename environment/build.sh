#!/usr/bin/env bash
# Compile the y5_compositor binary for a chosen backend and profile.
#
# Usage: ./build.sh [winit|udev|native] [debug|release]   (default: winit debug)
#   winit        : nested backend — runs inside an existing Wayland/X session.
#                  Plain `cargo build` (default `backend-winit` feature).
#   udev|native  : DRM/KMS native backend — runs on real hardware / a TTY.
#                  `cargo build --no-default-features --features backend-native`.
#   debug | release : cargo profile (release adds --release).
#
# The two axes are independent: any backend can be built in either profile. The
# backend is selected at COMPILE time via the `backend-winit` (default) /
# `backend-native` cargo features, which `main.rs` switches on with
# `#[cfg(feature = "backend-native")]`.
#
# Prints the path to the built binary as the only stdout line (build logs go to
# stderr), so callers can do:  BIN="$(./build.sh udev release)"
#
# Debug builds trim debug info to line tables only — keeps backtrace line numbers,
# much faster links + smaller binary.
#
# Env overrides:
#   Y5_TARGET_DIR  cargo target dir (default: the loader workspace's own target/)
#   Y5_REPO_ROOT   repo root (default: auto-detected by walking up to a compositor* dir)
# Note: rustflags (warnings) live in .cargo/config.toml. Do NOT set RUSTFLAGS
# here — it would replace that config wholesale.
set -euo pipefail

BACKEND="${1:-winit}"
PROFILE="${2:-debug}"

# --- Locate the repo root --------------------------------------------------
# Nearest ancestor of this script that contains compositor* workspaces. Works on
# the host (script in environment/) and in the container (script at the repo root).
SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${Y5_REPO_ROOT:-}"
if [ -z "$REPO_ROOT" ]; then
    d="$SELF_DIR"
    while [ "$d" != "/" ]; do
        # Match a workspace DIRECTORY (a compositor*/ with a Cargo.toml), not just
        # any compositor*-named path — otherwise a stray file like
        # environment/compositor-env.sh falsely resolves the repo root to environment/.
        if compgen -G "$d/compositor*/Cargo.toml" >/dev/null 2>&1; then REPO_ROOT="$d"; break; fi
        d="$(dirname "$d")"
    done
fi
[ -n "$REPO_ROOT" ] || { echo "build.sh: could not locate repo root (no compositor* dir found)" >&2; exit 1; }

# --- Workspace conformance gate (layout/naming/size; see document/ARCHITECTURE.md) ---
# Skipped when node is unavailable (e.g. minimal container images) or Y5_SKIP_LINT is set.
# The distro bundle images set Y5_SKIP_LINT=1: they now carry node (for the Tauri devtool), but
# the authoritative lint already runs in ci.yml — re-running it here only risks the image's
# packaged node choking on the script, which must not fail a binary build.
if [ -z "${Y5_SKIP_LINT:-}" ] && [ -f "$REPO_ROOT/workspace.lint.js" ] && command -v node >/dev/null 2>&1; then
    ( cd "$REPO_ROOT" && node workspace.lint.js 2>&1 | tail -n 1 >&2 ) || { echo "build.sh: workspace.lint failed — run 'node workspace.lint.js' for details" >&2; exit 1; }
fi

# --- Locate the entry crate (rename-proof: keyed on the [[bin]] name) -------
EXECUTE_DIR="$(dirname "$(grep -rl --include=Cargo.toml --exclude-dir=target --exclude-dir=node_modules 'name *= *"y5_compositor"' "$REPO_ROOT"/compositor* | head -n1)")"
[ -n "$EXECUTE_DIR" ] && [ -d "$EXECUTE_DIR" ] || { echo "build.sh: could not find the y5_compositor crate" >&2; exit 1; }

# --- Backend -> cargo feature ----------------------------------------------
feature_args=()
case "$BACKEND" in
    winit) ;;                                       # default build (backend-winit), no extra feature
    udev|native)  feature_args=(--no-default-features --features backend-native) ;;
    *) echo "build.sh: unknown backend '$BACKEND' (expected winit|udev|native)" >&2; exit 1 ;;
esac

# --- Profile -> --release + target subdir ----------------------------------
case "$PROFILE" in
    debug)   profile_args=()          ; sub=debug   ;;
    release) profile_args=(--release) ; sub=release ;;
    *) echo "build.sh: unknown profile '$PROFILE' (expected debug|release)" >&2; exit 1 ;;
esac

# --- Target dir: explicit override, else the loader workspace's own target/ -
target_args=()
if [ -n "${Y5_TARGET_DIR:-}" ]; then
    TARGET_DIR="$Y5_TARGET_DIR"
    target_args=(--target-dir="$TARGET_DIR")
else
    ws_root="$EXECUTE_DIR"
    while [ "$ws_root" != "/" ] && ! grep -qs '^\[workspace\]' "$ws_root/Cargo.toml"; do
        ws_root="$(dirname "$ws_root")"
    done
    TARGET_DIR="$ws_root/target"   # cargo's default for this workspace
fi

# Debug profile: line-tables-only debug info → fast links, small binary, line numbers
# preserved in backtraces. Release already builds without debug info.
if [ "$PROFILE" = "debug" ]; then
    export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-line-tables-only}"
    export CARGO_PROFILE_DEV_SPLIT_DEBUGINFO="${CARGO_PROFILE_DEV_SPLIT_DEBUGINFO:-unpacked}"
fi

echo ">> building y5_compositor [backend=$BACKEND profile=$PROFILE]" >&2
( cd "$EXECUTE_DIR" && cargo build "${profile_args[@]}" "${feature_args[@]}" "${target_args[@]}" >&2 )

BIN="$TARGET_DIR/$sub/y5_compositor"
chmod +x "$BIN"
echo "$BIN"

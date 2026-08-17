#!/usr/bin/env bash
# Compile the y5_compositor binary for a chosen backend, and optionally install it.
#
# Usage: ./build.sh [winit|udev|native] [--deploy[=NAME]]
#   winit        : nested backend — runs inside an existing Wayland/X session.
#                  Plain `cargo build` (default `backend-winit` feature).
#   udev|native  : DRM/KMS native backend — runs on real hardware / a TTY.
#                  `cargo build --no-default-features --features backend-native`.
#   --deploy         install to /usr/bin/y5.compositor
#   --deploy=NAME    install to /usr/bin/NAME   (e.g. --deploy=y5.compositor.exp)
#
# ALWAYS the `release-fast` profile: release optimizations without LTO. There is no
# profile argument, on purpose.
#   - `debug` is gone. It was the default, and its dependency tree costs ~4x the
#     release-fast one (measured: 14 GB vs 3.6 GB) for a compositor nobody steps
#     through. `release-fast` inherits `debug = "line-tables-only"`, so backtraces
#     still carry line numbers.
#   - fat-LTO `release` is `build-optimized.sh` and nothing else. Its final link is
#     serial and takes 10-20 minutes; it belongs to release tags, not to iteration.
#
# The backend is selected at COMPILE time via the `backend-winit` (default) /
# `backend-native` cargo features, which `main.rs` switches on with
# `#[cfg(feature = "backend-native")]`.
#
# Prints the path to the built binary as the only stdout line (build logs go to
# stderr), so callers can do:  BIN="$(./build.sh udev)"
#
# Env overrides:
#   Y5_TARGET_DIR  cargo target dir. Default: the repo-wide one pinned by
#                  `.cargo/config.toml` ([build] target-dir), which is this
#                  workspace's own target/. Set it only when a build must NOT share
#                  that tree — the containers (/y5-target) and the cross compiler do.
#   Y5_REPO_ROOT   repo root (default: auto-detected by walking up to a compositor* dir)
#   Y5_SKIP_LINT   skip the workspace conformance gate
#   Y5_PROFILE     INTERNAL. `build-optimized.sh` sets this to `release` so both
#                  scripts share one code path. Not a user-facing knob — if you want
#                  fat LTO, run build-optimized.sh so the intent is on the command line.
# Note: rustflags (warnings, target-cpu) live in .cargo/config.toml. Do NOT set
# RUSTFLAGS here — it would replace that config wholesale.
set -euo pipefail

BACKEND=winit
DEPLOY=""          # unset = build only; otherwise the /usr/bin name to install as
for arg in "$@"; do
    case "$arg" in
        winit | udev | native) BACKEND="$arg" ;;
        --deploy)   DEPLOY="y5.compositor" ;;
        --deploy=*) DEPLOY="${arg#--deploy=}" ;;
        -h | --help)
            sed -n '2,10p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            echo
            echo "Always the release-fast profile. For fat-LTO release builds:"
            echo "  environment/build-optimized.sh [winit|udev|native] [--deploy[=NAME]]"
            exit 0 ;;
        debug | release | fast | release-fast)
            echo "build.sh: profiles are gone — this script is always release-fast." >&2
            echo "          For the fat-LTO release build, use: environment/build-optimized.sh $BACKEND" >&2
            exit 1 ;;
        *) echo "build.sh: unknown arg '$arg' (see --help)" >&2; exit 1 ;;
    esac
done
[ -z "$DEPLOY" ] || case "$DEPLOY" in
    */*) echo "build.sh: --deploy takes a BINARY NAME, not a path ('$DEPLOY')" >&2; exit 1 ;;
esac

# --- Locate the repo root --------------------------------------------------
# Nearest ancestor of this script that contains compositor* workspaces. Works on
# the host (script in environment/) and in the container (script at the repo root).
#
# The marker MUST be a committed file. Every Cargo.toml in the linked tree is a
# generated artifact (see below), so a fresh clone — every CI checkout — has none:
# testing for `compositor*/Cargo.toml` first is a chicken-and-egg that fails the
# build before the generator that would create them ever runs.
# workspace.catalog.json is the authored source the generator reads, it lives only
# at the repo root, and it cannot be mistaken for a stray `compositor*`-named file
# like environment/compositor-env.sh. The Cargo.toml glob stays as a fallback for
# trees that ship the generated manifests without compositor.workspace/.
SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${Y5_REPO_ROOT:-}"
if [ -z "$REPO_ROOT" ]; then
    d="$SELF_DIR"
    while [ "$d" != "/" ]; do
        if [ -f "$d/compositor.workspace/workspace.catalog.json" ] \
            || compgen -G "$d/compositor*/Cargo.toml" >/dev/null 2>&1; then REPO_ROOT="$d"; break; fi
        d="$(dirname "$d")"
    done
fi
[ -n "$REPO_ROOT" ] || { echo "build.sh: could not locate repo root (no compositor* dir found)" >&2; exit 1; }

# --- Generate the Cargo manifests ------------------------------------------
# Every Cargo.toml in the linked tree is a build artifact, generated from
# vendor.catalog.json + workspace.catalog.json + the per-crate crate.json, and
# gitignored. A fresh clone has NO manifests at all, so this has to run before
# cargo — not as a command anyone has to remember. That is also what retires the
# old "forgot to run link.all.sh" footgun: there is no committed generated block
# left to go stale.
#
# It also has to run before the LINT below, which reads each root's generated
# Cargo.toml (member globs, workspace-dependency table) and would abort on a fresh
# clone that has none.
#
# Skipped when node is unavailable: the distro bundle images build from a tree that
# was generated before the image was built.
if [ -f "$REPO_ROOT/compositor.workspace/workspace.generate.js" ] && command -v node >/dev/null 2>&1; then
    ( cd "$REPO_ROOT" && node compositor.workspace/workspace.generate.js >/dev/null ) \
        || { echo "build.sh: workspace.generate failed" >&2; exit 1; }
fi

# --- Workspace conformance gate (layout/naming/size; see CLAUDE.md) ---------
# Skipped when node is unavailable (e.g. minimal container images) or Y5_SKIP_LINT is set.
# The distro bundle images set Y5_SKIP_LINT=1: they now carry node (for the Tauri devtool), but
# the authoritative lint already runs in ci.yml — re-running it here only risks the image's
# packaged node choking on the script, which must not fail a binary build.
if [ -z "${Y5_SKIP_LINT:-}" ] && [ -f "$REPO_ROOT/compositor.workspace/workspace.lint.js" ] && command -v node >/dev/null 2>&1; then
    ( cd "$REPO_ROOT" && node compositor.workspace/workspace.lint.js 2>&1 | tail -n 1 >&2 ) || { echo "build.sh: workspace.lint failed — run 'node compositor.workspace/workspace.lint.js' for details" >&2; exit 1; }
fi

# --- Locate the entry crate (rename-proof: keyed on the [[bin]] name) -------
EXECUTE_DIR="$(dirname "$(grep -rl --include=Cargo.toml --exclude-dir=target --exclude-dir=node_modules 'name *= *"y5_compositor"' "$REPO_ROOT"/compositor* | head -n1)")"
[ -n "$EXECUTE_DIR" ] && [ -d "$EXECUTE_DIR" ] || { echo "build.sh: could not find the y5_compositor crate" >&2; exit 1; }

# --- Backend -> cargo feature ----------------------------------------------
feature_args=()
case "$BACKEND" in
    winit) ;;                                       # default build (backend-winit), no extra feature
    udev|native)  feature_args=(--no-default-features --features backend-native) ;;
esac

# --- Profile ---------------------------------------------------------------
# release-fast unless build-optimized.sh asked for the fat-LTO release.
case "${Y5_PROFILE:-release-fast}" in
    release-fast) profile_args=(--profile release-fast) ; sub=release-fast ;;
    release)      profile_args=(--release)              ; sub=release      ;;
    *) echo "build.sh: bad Y5_PROFILE '${Y5_PROFILE}' (internal; expected release-fast|release)" >&2; exit 1 ;;
esac

# --- Target dir ------------------------------------------------------------
# Normally NOT passed: `.cargo/config.toml` pins one target dir for the whole repo,
# and this workspace's own target/ IS that dir. Only an explicit Y5_TARGET_DIR (the
# containers, the cross compiler) sends the build somewhere else.
target_args=()
if [ -n "${Y5_TARGET_DIR:-}" ]; then
    TARGET_DIR="$Y5_TARGET_DIR"
    target_args=(--target-dir="$TARGET_DIR")
else
    ws_root="$EXECUTE_DIR"
    while [ "$ws_root" != "/" ] && ! grep -qs '^\[workspace\]' "$ws_root/Cargo.toml"; do
        ws_root="$(dirname "$ws_root")"
    done
    TARGET_DIR="$ws_root/target"   # == the pinned [build] target-dir
fi

echo ">> building y5_compositor [backend=$BACKEND profile=$sub]" >&2
( cd "$EXECUTE_DIR" && cargo build "${profile_args[@]}" "${feature_args[@]}" "${target_args[@]}" >&2 )

BIN="$TARGET_DIR/$sub/y5_compositor"
chmod +x "$BIN"

# CAP_SYS_NICE on the binary lets settings.json `priority="auto"` take the
# direct-nice rung (no rtkit round trip). Best-effort and re-applied every
# build — cargo rewrites the binary, which clears file capabilities.
# Interactive (stderr is a tty): plain sudo, which may prompt for a password
# on /dev/tty. Unattended (CI/container/cron): `sudo -n` never prompts, so a
# headless build can't hang; without the cap the compositor simply falls back
# to rtkit over D-Bus.
setcap_best_effort() {
    command -v setcap >/dev/null 2>&1 || [ -x /usr/sbin/setcap ] || return 0
    if [ -t 2 ]; then
        sudo setcap cap_sys_nice+ep "$1" \
            || echo ">> note: setcap cap_sys_nice failed; priority=\"auto\" will use rtkit" >&2
    else
        sudo -n setcap cap_sys_nice+ep "$1" 2>/dev/null \
            || echo ">> note: setcap cap_sys_nice skipped (unattended, needs passwordless sudo); priority=\"auto\" will use rtkit" >&2
    fi
}
setcap_best_effort "$BIN"

# --- Deploy ----------------------------------------------------------------
# COPY, not move: a move takes the binary out of target/, so the next build has to
# relink the whole thing. The capability is re-applied because install/cp drops it.
if [ -n "$DEPLOY" ]; then
    echo ">> installing -> /usr/bin/$DEPLOY" >&2
    sudo install -m 755 "$BIN" "/usr/bin/$DEPLOY"
    setcap_best_effort "/usr/bin/$DEPLOY"
fi

echo "$BIN"

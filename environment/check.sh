#!/usr/bin/env bash
# Does it compile? — the workspace conformance lint, then `cargo check` in every
# workspace root (or just one).
#
# Usage: ./check.sh [workspace-dir]
#   no argument     lint gate, then check every workspace root
#   workspace-dir   check only that root, e.g. ./check.sh compositor.orchestration
#                   (a path to a dir whose Cargo.toml has a [workspace] table).
#                   Skips the repo-wide lint — it is a whole-tree gate, and the
#                   point of naming one root is a fast answer. `build.sh` and the
#                   no-argument form still run it.
#
# THIS is the compile check. Not `cargo check` by hand: the repo is 30 independent
# workspaces, so a bare `cargo check` run in each of them builds a complete
# dependency tree PER ROOT. That is how a checkout reaches 100+ GB, and this script
# used to be the thing doing it.
#
# Two properties make it cheap now:
#   - ONE target dir, the same one `environment/build.sh` writes to. Pinned repo-wide
#     by `.cargo/config.toml` and re-exported here so the guarantee does not depend
#     on that file being found.
#   - the SAME profile as build.sh (release-fast), so a check after a build reuses
#     the artifacts already on disk instead of growing a second tree beside them.
#
# Env overrides:
#   Y5_TARGET_DIR  cargo target dir (default: the loader workspace's target/)
#   Y5_REPO_ROOT   repo root (default: auto-detected)
set -euo pipefail

# --help, like build.sh / build-optimized.sh / run-host.sh. Without this the flag
# falls through to the workspace-dir argument and reports "'--help' is not a
# workspace root", which is a confusing answer to a reasonable question.
case "${1:-}" in
    -h|--help) sed -n '2,11p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# --- Locate the repo root --------------------------------------------------
# Same test as build.sh, and for the same reason: the marker has to be a COMMITTED
# file, because every Cargo.toml here is generated and a fresh clone has none. A
# bare `compositor*` glob would resolve the root to environment/ (there is a
# compositor-env.sh in it), so match workspace.catalog.json — the authored source
# the generator reads — and keep the Cargo.toml glob only as a fallback.
REPO_ROOT="${Y5_REPO_ROOT:-}"
if [ -z "$REPO_ROOT" ]; then
    d="$SELF_DIR"
    while [ "$d" != "/" ]; do
        if [ -f "$d/compositor.workspace/workspace.catalog.json" ] \
            || compgen -G "$d/compositor*/Cargo.toml" >/dev/null 2>&1; then REPO_ROOT="$d"; break; fi
        d="$(dirname "$d")"
    done
fi
[ -n "$REPO_ROOT" ] || { echo "check.sh: could not locate repo root" >&2; exit 1; }

# --- Generate the Cargo manifests ------------------------------------------
# Ahead of everything that reads a manifest — the target-dir resolution right
# below greps Cargo.toml for the y5_compositor [[bin]] and then for `[workspace]`,
# and a fresh clone has no manifests at all: they are generated artifacts (see
# build.sh).
if [ -f "$REPO_ROOT/compositor.workspace/workspace.generate.js" ] && command -v node >/dev/null 2>&1; then
    ( cd "$REPO_ROOT" && node compositor.workspace/workspace.generate.js >/dev/null ) \
        || { echo "check.sh: workspace.generate failed" >&2; exit 1; }
fi

# --- The one shared target dir ---------------------------------------------
# Resolved the way build.sh resolves it: the workspace root above the crate that
# declares the y5_compositor [[bin]]. Exported (not passed per-invocation) so every
# cargo below inherits it, whatever it decides to do.
if [ -n "${Y5_TARGET_DIR:-}" ]; then
    TARGET_DIR="$Y5_TARGET_DIR"
else
    execute_dir="$(dirname "$(grep -rl --include=Cargo.toml --exclude-dir=target --exclude-dir=node_modules 'name *= *"y5_compositor"' "$REPO_ROOT"/compositor* | head -n1)")"
    [ -n "$execute_dir" ] && [ -d "$execute_dir" ] || { echo "check.sh: could not find the y5_compositor crate" >&2; exit 1; }
    ws_root="$execute_dir"
    while [ "$ws_root" != "/" ] && ! grep -qs '^\[workspace\]' "$ws_root/Cargo.toml"; do
        ws_root="$(dirname "$ws_root")"
    done
    TARGET_DIR="$ws_root/target"
fi
export CARGO_TARGET_DIR="$TARGET_DIR"

# --- Which roots -----------------------------------------------------------
roots=()
if [ $# -gt 0 ]; then
    case "$1" in
        -h|--help) sed -n '2,11p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    esac
    one="$1"
    [ -d "$one" ] || one="$REPO_ROOT/$1"
    [ -f "$one/Cargo.toml" ] && grep -qs '^\[workspace\]' "$one/Cargo.toml" \
        || { echo "check.sh: '$1' is not a workspace root (no Cargo.toml with [workspace])" >&2; exit 1; }
    roots=("$(cd "$one" && pwd)")
else
    echo ">> workspace.lint" >&2
    # --deps runs the cargo-shear-backed deps-unused rule. Opt-in because it costs
    # minutes; this script is already paying for a full compile, build.sh is not.
    ( cd "$REPO_ROOT" && node compositor.workspace/workspace.lint.js --deps >&2 )
    while IFS= read -r ws; do
        roots+=("$(dirname "$ws")")
    done < <(grep -l '^\[workspace\]' "$REPO_ROOT"/compositor*/Cargo.toml "$REPO_ROOT"/compositor*/*/Cargo.toml 2>/dev/null)
fi

echo ">> target dir: $CARGO_TARGET_DIR" >&2

fail=0
for ws_dir in "${roots[@]}"; do
    echo ">> cargo check: ${ws_dir#"$REPO_ROOT"/}" >&2
    # release-fast, matching build.sh — a check right after a build is then almost
    # free, because the artifacts are already there.
    ( cd "$ws_dir" && cargo check --quiet --profile release-fast </dev/null ) || fail=1
done
exit $fail

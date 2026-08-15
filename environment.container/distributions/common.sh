#!/usr/bin/env bash
# Shared helpers for the per-distro dev loop (image.sh / run.sh / build.sh).
# Source this; it defines REPO_ROOT, ENV_CONTAINER and the distro_* helpers.
#
# A "distro" is just a subdirectory here that contains a Containerfile — drop in a
# new <name>/Containerfile and it is picked up automatically (no edits here).

_DIST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# distributions/ -> environment.container/ -> repo root
REPO_ROOT="$(cd "$_DIST_DIR/../.." && pwd)"
# Reuse the containerized run's curated GPU/session env (NVIDIA EGL paths,
# COMPOSITOR_RENDER_NODE, WAYLAND_DISPLAY=wayland-host, …).
ENV_CONTAINER="$_DIST_DIR/../container.env"

# Print the available distros (one per line): every subdir with a Containerfile.
distro_list() {
    local d
    for d in "$_DIST_DIR"/*/; do
        [ -f "$d/Containerfile" ] && basename "$d"
    done
}

# Exit with an error unless $1 names a distro that has a Containerfile.
distro_validate() {
    local want="$1" d
    for d in $(distro_list); do
        [ "$d" = "$want" ] && return 0
    done
    echo "unknown distro '$want' — have: $(distro_list | tr '\n' ' ')" >&2
    exit 1
}

# Path to the materialized source tree the distro builds use: a fresh, self-contained clone
# of the repo (committed files only — no target/ dirs, no untracked cruft, a real .git/ with
# NO external worktree link). Override with Y5_DIST_SRC. Produced by prepare_source below.
DIST_SRC="${Y5_DIST_SRC:-$_DIST_DIR/.src}"

# Per-distro cargo target cache, persisted on the host and mounted into the build at /y5-target.
# Keeps the compiled dependency graph warm so a CHANGED clone rebuilds incrementally instead of
# from scratch. One dir per distro (different distros have incompatible ABIs/toolchains, so they
# must not share). Override the base with Y5_DIST_CACHE.
DIST_CACHE="${Y5_DIST_CACHE:-$_DIST_DIR/.cache}"

# Materialize DIST_SRC by cloning the local repo. This MUST run where the repo's FULL git data
# exists. In a linked git worktree, .git is a file pointing at an external gitdir
# (e.g. /home/y5/nourish/.git) — a machine that only has the worktree's files (the host) can't
# resolve that link, so it can't clone. Run this in the sandbox/dev env that has the whole repo;
# the result is a normal self-contained repo that image.sh/build.sh/run.sh build from ANYWHERE,
# host included. Refreshes (removes) any existing DIST_SRC.
prepare_source() {
    rm -rf "$DIST_SRC"
    mkdir -p "$(dirname "$DIST_SRC")"
    # --no-hardlinks: DIST_SRC may be on a different filesystem than the repo, where git's
    # default object-hardlinking fails ("Invalid cross-device link"); copying always works.
    if ! git -C "$REPO_ROOT" clone --quiet --no-hardlinks . "$DIST_SRC" >&2; then
        echo "prepare_source: 'git clone' of $REPO_ROOT failed." >&2
        echo "  Run this where the repo's full git data exists (the dev sandbox): a linked" >&2
        echo "  worktree's .git points at $(git -C "$REPO_ROOT" rev-parse --git-common-dir 2>/dev/null || echo '<external gitdir>')," >&2
        echo "  which a host that only has the worktree files cannot resolve." >&2
        rm -rf "$DIST_SRC"
        return 1
    fi
}

# Image tag for <distro> <profile> (debug/release kept as separate images).
# Tag: y5-distro-<distro>-<mode>. Keyed on the build MODE (bundle|devloop) so a
# dev image and a release bundle can never collide on one tag.
distro_image() { printf 'y5-distro-%s-%s' "$1" "${2:?distro_image: mode required}"; }
# Container name for a running winit session on <distro>.
distro_container() { printf 'y5-distro-%s' "$1"; }

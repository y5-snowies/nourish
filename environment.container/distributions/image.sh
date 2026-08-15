#!/usr/bin/env bash
# Build the per-distro image for a target distribution. The Containerfile installs that
# distro's build deps, then CLONES the committed tree from the local git repo (bind-mounted
# at /repo — no COPY of the live workspace) and compiles the winit y5_compositor binary,
# stashing it at /usr/local/bin/y5_compositor inside the image.
#
# Usage: ./image.sh <distro> <bundle|devloop>
#   distro: a subdir here with a Containerfile (e.g. fedora, ubuntu, arch)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$HERE/common.sh"

DISTRO="${1:?usage: image.sh <distro> <bundle|devloop>  (have: $(distro_list | tr '\n' ' ')) }"
MODE="${2:-}"

# MODE is REQUIRED — see the Containerfile. `bundle` is the shipped install bundle
# (udev, fat LTO, via prepare.sh); `devloop` is the nested winit dev image. No default:
# a release path that forgets to say `bundle` must fail, not silently produce a dev image.
case "$MODE" in
    bundle|devloop) ;;
    *) echo "$(basename "$0"): MODE must be 'bundle' or 'devloop' (got: '$MODE')" >&2; exit 1 ;;
esac

distro_validate "$DISTRO"

IMAGE="$(distro_image "$DISTRO" "$MODE")"

# The prepared self-contained clone (DIST_SRC) is bind-mounted into the build at /repo via
# `-v` (a reliable buildah feature — unlike the inline `--mount=type=bind,source=.` context
# mount, which podman does not populate from the build context). The Containerfile then
# `git clone /repo`s it. DIST_SRC has no external worktree link, so this builds on the host
# too. If it isn't there yet, materialize it now (only works where the git data exists, e.g.
# the sandbox). The build context itself is tiny (just the Containerfile dir) — the source
# arrives through the -v mount, not the context.
if [ ! -d "$DIST_SRC/.git" ]; then
    echo ">> no prepared source at $DIST_SRC — materializing (needs full git data) ..." >&2
    prepare_source
fi

# Per-distro cargo target cache (persists on the host across rebuilds → incremental compile
# when the clone changes). Mounted at /y5-target; the Containerfile sets Y5_TARGET_DIR to match.
TARGET_CACHE="$DIST_CACHE/$DISTRO"
mkdir -p "$TARGET_CACHE"

# Extra `podman build` args (e.g. `--build-arg VERSION=x`) can be injected
# via Y5_EXTRA_BUILD_ARGS — the multiarch-publish CD uses this to build the full install bundle
# instead of the dev-loop winit binary. Word-split into an array so each token is its own arg.
read -ra _EXTRA_BUILD_ARGS <<< "${Y5_EXTRA_BUILD_ARGS:-}"

echo ">> building $IMAGE  (distro=$DISTRO mode=$MODE, source $DIST_SRC, target cache $TARGET_CACHE)" >&2
podman build \
    --build-arg MODE="$MODE" \
    "${_EXTRA_BUILD_ARGS[@]}" \
    -v "$DIST_SRC:/repo:ro" \
    -v "$TARGET_CACHE:/y5-target" \
    --security-opt label=disable \
    -t "$IMAGE" \
    -f "$HERE/$DISTRO/Containerfile" \
    "$HERE/$DISTRO"

echo ">> built $IMAGE" >&2

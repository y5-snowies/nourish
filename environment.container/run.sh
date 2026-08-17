#!/usr/bin/env bash
# Build + run the compositor inside the dev container, under the host's nested Wayland.
# Usage: ./run.sh [winit|udev]   (default: winit)
# Always release-fast — build.sh has no profile argument (see environment/build.sh).
set -euo pipefail

BACKEND="${1:-winit}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/.." && pwd)"

IMAGE=y5-compositor-smithay-dev
NAME=y5-compositor-smithay-dev
TARGET_DIR="$HERE/run.cargo.target"
mkdir -p "$TARGET_DIR"

# Mount every top-level Cargo workspace (compositor*) plus the vendored deps, so adding
# or renaming a workspace needs no edit here.
mounts=()
for ws in "$REPO_ROOT"/compositor* "$REPO_ROOT/vendor"; do
    mounts+=( -v "$ws:/working.directory/$(basename "$ws"):Z" )
done

podman rm -f -t 0 "$NAME" 2>/dev/null || true
podman run -it --rm \
    --name "$NAME" \
    --init \
    --network anvil-test \
    --env-file "$HERE/container.env" \
    --device nvidia.com/gpu=all \
    -v "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY:/tmp/wayland-host:ro" \
    "${mounts[@]}" \
    -v "$TARGET_DIR:/working.directory/run.cargo.target" \
    -v "$REPO_ROOT/.cargo/config.toml:/working.directory/.cargo/config.toml:ro,Z" \
    -v "$HERE/../environment/build.sh:/working.directory/build.sh:Z" \
    -v "$HERE/entrypoint.sh:/working.directory/entrypoint.sh:Z" \
    -v "$HERE/../environment/compositor-env.sh:/working.directory/compositor-env.sh:Z" \
    -w /working.directory \
    --security-opt label=disable \
    --entrypoint=/bin/bash \
    "$IMAGE" /working.directory/entrypoint.sh "$BACKEND"

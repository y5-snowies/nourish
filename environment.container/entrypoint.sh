#!/usr/bin/env bash
# In-container build + run of y5_compositor. Invoked by run.sh as the container
# entrypoint (build.sh is mounted alongside it) — not meant to be run on the host.
# Usage: entrypoint.sh [winit|udev]   (default: winit)
set -euo pipefail

BACKEND="${1:-winit}"

# Persist the build cache in the host-mounted target dir, and compile via the shared
# build.sh so the backend/profile logic lives in exactly one place.
export Y5_TARGET_DIR=/working.directory/run.cargo.target
BIN="$(/working.directory/build.sh "$BACKEND")"

# Collapse the individual COMPOSITOR_* knobs (from container.env / --env-file) into
# the settings file the compositor reads.
# shellcheck disable=SC1091
. /working.directory/compositor-env.sh
compositor_write_settings

exec "$BIN"

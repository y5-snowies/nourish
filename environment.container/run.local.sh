#!/usr/bin/env bash
# Build and run the compositor LOCALLY (no container) under the host's nested Wayland,
# using the curated GPU/session environment from container.env (NVIDIA EGL paths,
# COMPOSITOR_RENDER_NODE, WAYLAND_DISPLAY, etc.). The containerized equivalent is run.sh.
#
# Usage: ./run.local.sh [winit|udev]   (default: winit)
# Always release-fast — build.sh has no profile argument.
#   COMPOSITOR_LOG_LEVEL is honored if already set, else defaults to all levels.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND="${1:-winit}"

# Build first; build.sh prints the binary path on stdout (logs go to stderr).
BIN="$("$HERE/../environment/build.sh" "$BACKEND")"

# Load the same environment the container uses (render node, EGL vendor, wayland display…).
set -a
# shellcheck disable=SC1091
. "$HERE/container.env"
set +a

export COMPOSITOR_LOG_LEVEL="${COMPOSITOR_LOG_LEVEL:-info,warn,error,trace}"

# Collapse the individual COMPOSITOR_* knobs (incl. those from container.env) into
# the settings file the compositor reads.
# shellcheck disable=SC1091
. "$HERE/../environment/compositor-env.sh"
compositor_write_settings

echo ">> running $BIN  (render node: ${COMPOSITOR_RENDER_NODE:-default}, levels: $COMPOSITOR_LOG_LEVEL)" >&2
exec "$BIN"

#!/usr/bin/env bash
# Build a release udev y5_compositor on the host (via build.sh), then install or deploy it.
# Usage: ./build-release.sh <dev|system> [fast]
#   dev     -> sudo cp  to /usr/bin/y5.compositor.dev
#   system  -> sudo mv  to /usr/bin/y5.compositor
#   fast    -> use the release-fast profile: same optimizations but no LTO,
#              skipping the multi-minute serial link (~30% larger binary).
#
# Deployed builds always target real hardware, so this uses the udev backend in
# release (or release-fast) profile. For other combinations build directly with
# ./build.sh.
set -euo pipefail

DEST="${1:?usage: build-release.sh <dev|system> [fast]}"
PROFILE="${2:-release}"
case "$PROFILE" in
    release|fast) ;;
    *) echo "build-release.sh: unknown profile '$PROFILE' (expected fast, or omit for release)" >&2; exit 1 ;;
esac
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

BIN="$("$HERE/build.sh" udev "$PROFILE")"

# CAP_SYS_NICE on the installed binary: settings.json `priority="auto"` takes
# the direct-nice rung without the rtkit D-Bus round trip. (cp/mv drop file
# capabilities, so this must run after the install step.)
case "$DEST" in
    dev)    sudo cp "$BIN" /usr/bin/y5.compositor.dev
            sudo setcap cap_sys_nice+ep /usr/bin/y5.compositor.dev ;;
    system) sudo mv "$BIN" /usr/bin/y5.compositor
            sudo setcap cap_sys_nice+ep /usr/bin/y5.compositor ;;
    *) echo "unknown dest '$DEST' (expected dev|system)" >&2; exit 1 ;;
esac

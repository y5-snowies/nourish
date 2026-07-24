#!/usr/bin/env bash
# Build a release udev y5_compositor on the host (via build.sh), then install or deploy it.
# Usage: ./build-release.sh <dev|system|remote>
#   dev     -> sudo cp  to /usr/bin/y5.compositor.dev
#   system  -> sudo mv  to /usr/bin/y5.compositor
#   remote  -> scp      to y5@yrd.local:/home/y5/compositor
#
# Deployed builds always target real hardware, so this uses the udev backend in
# release profile. For other combinations build directly with ./build.sh.
set -euo pipefail

DEST="${1:?usage: build-release.sh <dev|system|remote>}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

BIN="$("$HERE/build.sh" udev release)"

# CAP_SYS_NICE on the installed binary: settings.json `priority="auto"` takes
# the direct-nice rung without the rtkit D-Bus round trip. (cp/mv drop file
# capabilities, so this must run after the install step.)
case "$DEST" in
    dev)    sudo cp "$BIN" /usr/bin/y5.compositor.dev
            sudo setcap cap_sys_nice+ep /usr/bin/y5.compositor.dev ;;
    system) sudo mv "$BIN" /usr/bin/y5.compositor
            sudo setcap cap_sys_nice+ep /usr/bin/y5.compositor ;;
    remote)
        scp "$BIN" y5@yrd.local:/home/y5/compositor
        # -t: interactive sudo password prompt on the remote.
        ssh -t y5@yrd.local 'chmod +x /home/y5/compositor && sudo setcap cap_sys_nice+ep /home/y5/compositor'
        ;;
    *) echo "unknown dest '$DEST' (expected dev|system|remote)" >&2; exit 1 ;;
esac

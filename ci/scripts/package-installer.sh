#!/usr/bin/env bash
# CD: build the full end-user INSTALL BUNDLE — the compositor (udev + dev binaries),
# the developer-tool window, the polkit agent, the MX gesture daemon and the
# interactive `y5-install` — by delegating to
# compositor.installer/prepare.sh. Leaves the bundle in dist/ for the publish jobs:
#   - GitHub/GitLab Pages, served at /release/latest/fedora44/ (what get.sh fetches)
#   - GitHub/GitLab Release assets (manual download)
#
# This is the single place that turns the tree into a shippable bundle; both platforms
# call it so the real logic lives once (see ci/README.md).
#
# Usage: package-installer.sh [extra prepare.sh args, e.g. --skip=devtool]
# Output: dist/package.tar.gz + dist/SHA256SUMS (paths printed to stdout).

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$REPO_ROOT"

out="$REPO_ROOT/dist"
log "building install bundle via compositor.installer/prepare.sh -> $out"
compositor.installer/prepare.sh --out="$out" "$@"

log "install bundle:"
printf '%s\n' "$out/package.tar.gz" "$out/SHA256SUMS"

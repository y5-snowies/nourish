#!/usr/bin/env bash
# CD step: build the release binary and bundle the downloadable artifacts. This is the
# real "deploy" for y5 — produce versioned, checksummed artifacts; live host deploy is a
# separate, optional, manual job (environment/build.sh --deploy).
#
# Usage: package-release.sh [version]
#   version defaults to the git tag (CI_COMMIT_TAG / GITHUB_REF_NAME) or the short SHA.
#
# Reuses environment/build-optimized.sh verbatim (fat LTO; it discovers the entry crate
# + workspace itself). Shipped artifacts get the optimized profile, never release-fast.
#
# udev ONLY. This used to also fat-LTO a winit binary into the tarball, which cost a
# second ~12 min serial link for a nested-session dev backend nobody installs — the
# end-user path is the installer bundle (compositor.installer/prepare.sh), and that
# ships udev. build-optimized.sh now refuses winit outright.
# Output: dist/y5-compositor-<version>.tar.gz + dist/SHA256SUMS, paths printed to stdout.

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
cd "$REPO_ROOT"

version="${1:-${CI_COMMIT_TAG:-${GITHUB_REF_NAME:-$(git rev-parse --short HEAD)}}}"
dist="$REPO_ROOT/dist"
stage="$dist/y5-compositor-$version"
rm -rf "$dist"; mkdir -p "$stage"

log "building udev release binary (via environment/build-optimized.sh)"
udev_bin="$(environment/build-optimized.sh udev)"
cp "$udev_bin" "$stage/y5_compositor"

# Fold in docs + coverage if earlier stages produced them (optional).
[ -d "$REPO_ROOT/public" ] && cp -r "$REPO_ROOT/public" "$stage/docs"
[ -f "$REPO_ROOT/.ci-coverage/coverage.lcov" ] && cp "$REPO_ROOT/.ci-coverage/coverage.lcov" "$stage/"
[ -f "$REPO_ROOT/.ci-coverage/coverage.txt" ]  && cp "$REPO_ROOT/.ci-coverage/coverage.txt"  "$stage/"

cat > "$stage/RELEASE.txt" <<EOF
y5_compositor release $version
commit: $(git rev-parse HEAD)
built:  $(git log -1 --format=%cI HEAD)
backend:  udev (y5_compositor)
EOF

tarball="$dist/y5-compositor-$version.tar.gz"
log "packaging $tarball"
tar -C "$dist" -czf "$tarball" "y5-compositor-$version"

( cd "$dist" && sha256sum "$(basename "$tarball")" > SHA256SUMS )

log "release artifacts:"
printf '%s\n' "$tarball" "$dist/SHA256SUMS"

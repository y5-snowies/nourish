#!/usr/bin/env bash
# Regenerate every Cargo manifest, then run the conformance lint.
#
# Kept as the familiar entry point, but it no longer has anything to maintain:
# `workspace.generate.js` discovers the roots from workspace.catalog.json, so the
# hand-written list of 18 `cd <root> && node ../workspace.link.js` invocations
# this script used to be — and which a new workspace root had to be added to by
# hand — is gone.
#
# You rarely need to run this. `environment/build.sh` and `environment/check.sh`
# generate before invoking cargo, so the manifests are never stale at build time.
# Run it when you want to refresh the tree for an editor without building.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

node "$HERE/workspace.generate.js"
node "$HERE/workspace.lint.js"

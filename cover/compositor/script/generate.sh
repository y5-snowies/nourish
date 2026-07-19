#!/usr/bin/env bash
#
# generate.sh — EMPTY step (see cover/README.md).
#
# Builds every crate in the project's workspace root(s) with LLVM instrumentation,
# enumerates the tests but runs NONE, then exports report/baseline.lcov: every member
# captured at 0%. lcov.info is set to that baseline (the report when no tests have run).
# This artifact is generated independently of any test run, guaranteeing its integrity.
#
# Project is inferred from this script's location (cover/<project>/script/).
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJ="$(basename "$(dirname "$DIR")")"
. "$DIR/../../lib/cover.sh"
cover_run "$PROJ" baseline "$@"

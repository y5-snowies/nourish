#!/usr/bin/env bash
#
# test.sh — TESTS + MERGE steps (see cover/README.md).
#
#   1. Ensure an EMPTY baseline exists (report/baseline.lcov). If missing it is built
#      first; set COVER_FORCE_BASELINE=1 to rebuild it.
#   2. Run the tests under cover/<project>/test/<fully-qualified-crate>/ (each *.rs file
#      is an integration test of that crate) -> report/tests.lcov.
#   3. MERGE baseline.lcov + tests.lcov (at the lcov level) -> report/lcov.info.
#
# The empty baseline is never mutated by a test run, so it keeps its integrity; the final
# report is a post-hoc merge. Project is inferred from this script's location.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJ="$(basename "$(dirname "$DIR")")"
. "$DIR/../../lib/cover.sh"
cover_run "$PROJ" test "$@"

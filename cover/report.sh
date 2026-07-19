#!/usr/bin/env bash
# Render all per-project lcov.info files into one self-contained cover/report.html.
# Run after generate.sh/test.sh. Requires node (used elsewhere in this repo).
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
node "$DIR/lib/report.js" "$@"
echo "cover: open file://$DIR/report.html" >&2

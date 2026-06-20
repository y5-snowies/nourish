#!/usr/bin/env bash
# Full-codebase coverage for ONE workspace entry, including dead code.
#
# Usage: coverage-full.sh <entry-path>     e.g. coverage-full.sh compositor.loader
#
# One canonical cargo-llvm-cov run does it all: `--all-targets` builds (and thus instruments)
# every target — lib, bins, tests, examples — so LLVM source-based coverage embeds a region
# for every line. Functions never executed (otherwise-dead / feature-gated code) are linked
# into the coverage map and reported at zero counts; the unit tests add real hit counts in
# the same pass; `--lcov --output-path` writes the merged lcov directly.
# (The old `--no-run` baseline phase is gone: it's deprecated and can't combine with
# `--no-report`, and the single-command form already covers untested code via `--all-targets`.)
#
# Per-entry lcov is written to  $REPO_ROOT/.ci-coverage/<slug>.lcov  (slug = entry with
# '/' and '.' turned into '_'). merge-coverage.sh later fuses all entries into one report.
#
# Feature axis: the entry that owns the y5_compositor [[bin]] is additionally built with
# --features backend-native so the DRM/KMS backend code is instrumented too. Override the
# whole feature set with Y5_COV_FEATURES if needed.

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

entry="${1:?usage: coverage-full.sh <entry-path>}"
[ -d "$REPO_ROOT/$entry" ] || die "no such entry: $entry"
command -v cargo-llvm-cov >/dev/null 2>&1 || cargo llvm-cov --version >/dev/null 2>&1 \
    || die "cargo-llvm-cov is required (cargo install cargo-llvm-cov)"

slug="$(printf '%s' "$entry" | tr '/.' '__')"
outdir="$REPO_ROOT/.ci-coverage"
mkdir -p "$outdir"
out="$outdir/$slug.lcov"

# Feature args: backend-native (the udev/DRM backend) for the entry that holds the bin.
feature_args=()
if [ -n "${Y5_COV_FEATURES:-}" ]; then
    # shellcheck disable=SC2206
    feature_args=(--features "$Y5_COV_FEATURES")
else
    bin_dir="$(y5_bin_crate_dir)"
    bin_ws="$(y5_workspace_root_of "$REPO_ROOT/$bin_dir")"
    if [ "$bin_ws" = "$REPO_ROOT/$entry" ]; then
        feature_args=(--features backend-native)
        log "$entry owns y5_compositor -> instrumenting the backend-native (udev/DRM) backend too"
    fi
fi

cd "$REPO_ROOT/$entry"

log "[$entry] clean coverage profile"
cargo llvm-cov clean --workspace

log "[$entry] instrument all targets, run tests, write lcov -> $out"
cargo llvm-cov --all-targets "${feature_args[@]}" --lcov --output-path "$out"

log "[$entry] coverage lcov written: $out"

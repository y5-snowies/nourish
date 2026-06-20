#!/usr/bin/env bash
# Fuse every per-entry lcov produced by coverage-full.sh into one repo-wide report.
#
# Inputs:  $REPO_ROOT/.ci-coverage/*.lcov
# Outputs (in $REPO_ROOT/.ci-coverage/):
#   coverage.lcov        merged lcov (shared path-deps merge by max hit count)
#   cobertura.xml        for GitLab MR line-coverage visualization
#   html/                browsable HTML report
#   coverage.txt         one-line total, e.g. "Coverage: 87.5%"  (also echoed to stdout)
#
# The printed "Coverage: NN.N%" line is what the GitLab `coverage:` keyword regex and the
# GitHub step summary scrape.

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

covdir="$REPO_ROOT/.ci-coverage"
mkdir -p "$covdir"

mapfile -t lcovs < <(find "$covdir" -maxdepth 1 -name '*.lcov' ! -name 'coverage.lcov' | sort)
# Coverage is informational — it must never block CI (and therefore promote/publish). If no
# per-entry lcovs arrived (e.g. a producing job failed, or none were uploaded), emit an
# "n/a" report and exit cleanly instead of failing the whole pipeline.
if [ "${#lcovs[@]}" -eq 0 ]; then
    log "WARNING: no per-entry *.lcov in $covdir — emitting an 'n/a' coverage report (not a gate)."
    printf 'Coverage: n/a\n' | tee "$covdir/coverage.txt"
    printf '_No coverage data was collected this run._\n' > "$covdir/coverage-crates.md"
    exit 0
fi

merged="$covdir/coverage.lcov"

log "merging ${#lcovs[@]} lcov file(s)"
add_args=()
for f in "${lcovs[@]}"; do add_args+=(-a "$f"); done
# --rc lcov_branch_coverage so branch data survives the merge; ignore mismatched-version
# noise across crates.
lcov "${add_args[@]}" -o "$merged" --ignore-errors inconsistent,format 2>/dev/null \
    || lcov "${add_args[@]}" -o "$merged"

log "converting to Cobertura"
if command -v lcov_cobertura >/dev/null 2>&1; then
    lcov_cobertura "$merged" --output "$covdir/cobertura.xml"
else
    log "lcov_cobertura not found — skipping cobertura.xml (install python lcov_cobertura)"
fi

log "generating HTML report"
genhtml "$merged" --output-directory "$covdir/html" --quiet \
    --ignore-errors inconsistent,format 2>/dev/null || genhtml "$merged" -o "$covdir/html" --quiet || true

# Total line coverage from lcov's own summary.
total="$(lcov --summary "$merged" 2>/dev/null | awk '/lines/{gsub("%","",$2); print $2; exit}')"
[ -n "$total" ] || total="0.0"
printf 'Coverage: %s%%\n' "$total" | tee "$covdir/coverage.txt"

# --- Per-crate breakdown (rendered ourselves; no third-party coverage service) ---------
# Aggregate the merged lcov by crate. A crate dir is the path segment before "/src/";
# paths are normalized to start at the first "compositor" component so the table reads in
# repo-relative terms. Emits a markdown table sorted by ascending coverage (worst first).
crates_md="$covdir/coverage-crates.md"
awk '
    /^SF:/ {
        f = substr($0, 4)
        i = index(f, "/src/"); if (i > 0) f = substr(f, 1, i - 1)
        j = index(f, "compositor"); if (j > 0) f = substr(f, j)
        cur = f
    }
    /^LF:/ { lf[cur] += substr($0, 4) + 0 }
    /^LH:/ { lh[cur] += substr($0, 4) + 0 }
    END {
        for (k in lf) {
            pct = (lf[k] > 0) ? (100.0 * lh[k] / lf[k]) : 0.0
            printf "%07.3f\t%s\t%d\t%d\t%.1f\n", pct, k, lh[k], lf[k], pct
        }
    }
' "$merged" | sort > "$covdir/.crates.tsv"

{
    echo "### 📊 Coverage by crate — total **${total}%** (dead code included)"
    echo
    echo "| Crate | Lines | Covered | Coverage |"
    echo "| --- | --: | --: | --: |"
    while IFS=$'\t' read -r _ crate hit found pct; do
        [ -n "$crate" ] || continue
        printf '| `%s` | %s | %s | %s%% |\n' "$crate" "$found" "$hit" "$pct"
    done < "$covdir/.crates.tsv"
} > "$crates_md"
rm -f "$covdir/.crates.tsv"
log "per-crate table -> $crates_md"

# --- Self-hosted SVG badge (shields-style; served from Pages, no external service) -----
int_total="${total%.*}"
if   [ "$int_total" -ge 90 ]; then color="#4c1"
elif [ "$int_total" -ge 75 ]; then color="#97ca00"
elif [ "$int_total" -ge 60 ]; then color="#a4a61d"
elif [ "$int_total" -ge 40 ]; then color="#dfb317"
elif [ "$int_total" -ge 20 ]; then color="#fe7d37"
else color="#e05d44"; fi
val="${total}%"
vw=$(( ${#val} * 7 + 10 ))   # rough value-box width
tw=$(( 61 + vw ))
cat > "$covdir/coverage.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" width="$tw" height="20" role="img" aria-label="coverage: $val">
  <linearGradient id="s" x2="0" y2="100%"><stop offset="0" stop-color="#bbb" stop-opacity=".1"/><stop offset="1" stop-opacity=".1"/></linearGradient>
  <clipPath id="r"><rect width="$tw" height="20" rx="3" fill="#fff"/></clipPath>
  <g clip-path="url(#r)">
    <rect width="61" height="20" fill="#555"/>
    <rect x="61" width="$vw" height="20" fill="$color"/>
    <rect width="$tw" height="20" fill="url(#s)"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">
    <text x="31" y="15" fill="#010101" fill-opacity=".3">coverage</text>
    <text x="31" y="14">coverage</text>
    <text x="$(( 61 + vw / 2 ))" y="15" fill="#010101" fill-opacity=".3">$val</text>
    <text x="$(( 61 + vw / 2 ))" y="14">$val</text>
  </g>
</svg>
SVG
log "coverage badge -> $covdir/coverage.svg"

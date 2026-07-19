#!/usr/bin/env bash
#
# cover/lib/cover.sh — shared coverage library for the y5 repo.
#
# Coverage model: LLVM source-based (region) coverage — the Rust analogue of V8's
# precise/"byte" coverage. rustc emits a coverage MAP (__llvm_covmap/__llvm_covfun)
# for every function/region when built with `-Cinstrument-coverage`; running the
# instrumented binaries writes execution COUNTS (.profraw); llvm-cov joins map+counts
# into lcov. With no tests, the map still enumerates every member and the counts are
# zero -> a valid lcov at 0% that captures all members. Add tests later and the exact
# same pipeline reports real executed coverage; no test lives in a shipping file.
#
# One lcov per PROJECT (a set of Cargo workspace roots — see cover/projects.json).
#
# Requires: rustc/cargo (stable), llvm-profdata + llvm-cov (version matching rustc's
#           LLVM — `rustc -vV | grep LLVM`), jq. Override tool paths with
#           LLVM_PROFDATA=/path LLVM_COV=/path if they are not on PATH.
#
# Not meant to be run directly — sourced by cover/<project>/script/{generate,test}.sh.

set -euo pipefail

COVER_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COVER_ROOT="$(cd "$COVER_LIB_DIR/.." && pwd)"      # cover/
REPO_ROOT="$(cd "$COVER_ROOT/.." && pwd)"          # repo root

LLVM_PROFDATA="${LLVM_PROFDATA:-llvm-profdata}"
LLVM_COV="${LLVM_COV:-llvm-cov}"

# Files under these paths are never our code; drop them from the report.
COVER_IGNORE_RE='(/vendor/|/registry/|/\.cargo/|/rustc/|/\.work/|/target/)'

cover_die() { echo "cover: error: $*" >&2; exit 1; }
cover_log() { echo "cover: $*" >&2; }

cover_check_tools() {
  command -v jq              >/dev/null || cover_die "jq not found"
  command -v cargo           >/dev/null || cover_die "cargo not found"
  command -v "$LLVM_PROFDATA" >/dev/null || cover_die "$LLVM_PROFDATA not found (dnf install llvm, or set LLVM_PROFDATA=)"
  command -v "$LLVM_COV"      >/dev/null || cover_die "$LLVM_COV not found (dnf install llvm, or set LLVM_COV=)"
}

# Echo every discovered workspace root (repo-relative), one per line, sorted.
# A workspace root = a dir at depth 1 or 2 under compositor.*/ whose Cargo.toml
# declares a [workspace] table.
cover_all_roots() {
  local d s
  {
    for d in "$REPO_ROOT"/compositor.*/; do
      if [ -f "$d/Cargo.toml" ] && grep -q '^\[workspace\]' "$d/Cargo.toml" 2>/dev/null; then
        echo "${d#"$REPO_ROOT"/}"
      fi
      for s in "$d"*/; do
        if [ -f "$s/Cargo.toml" ] && grep -q '^\[workspace\]' "$s/Cargo.toml" 2>/dev/null; then
          echo "${s#"$REPO_ROOT"/}"
        fi
      done
    done
  } 2>/dev/null | sed 's#/$##' | sort -u
}

# cover_roots <project> -> the project's workspace roots (repo-relative), one per line.
cover_roots() {
  local proj="$1" node
  node="$(jq -c --arg p "$proj" '.projects[$p] // empty' "$COVER_ROOT/projects.json")"
  [ -z "$node" ] && cover_die "unknown project '$proj' (see cover/projects.json)"

  if [ "$(printf '%s' "$node" | jq -r '.roots | type')" = "array" ]; then
    printf '%s' "$node" | jq -r '.roots[]'
    return
  fi
  [ "$(printf '%s' "$node" | jq -r '.roots')" = "*" ] || cover_die "bad 'roots' for project '$proj'"

  local excl; excl="$(printf '%s' "$node" | jq -r '.exclude[]?' | tr '\n' '|')"
  cover_all_roots | awk -v ex="$excl" '
    BEGIN { n = split(ex, a, "|"); for (i = 1; i <= n; i++) if (a[i] != "") skip[a[i]] = 1 }
    !($0 in skip)'
}

# Reorder a newline-separated root list cheapest-build-first. Cost proxy (no compile):
# the size of each root's TRANSITIVE dependency graph (cargo metadata package count),
# which tracks link/compile cost well — a root like kernel.loader that pulls the whole
# compositor sorts last even though its own manifest names no heavy crate. Roots that
# can't be resolved sort last. Affects build ORDER only, never the report's contents.
cover_order_roots() {
  local root abs n
  while IFS= read -r root; do
    [ -n "$root" ] || continue
    abs="$REPO_ROOT/$root"
    n="$( ( cd "$abs" && cargo metadata --format-version 1 2>/dev/null ) | jq '.packages | length' 2>/dev/null )"
    case "$n" in ''|*[!0-9]*) n=9999999 ;; esac   # unresolvable -> build last
    printf '%d\t%s\n' "$n" "$root"
  done <<< "$1" | sort -n -k1,1 | cut -f2-
}

# Setting RUSTFLAGS replaces ALL config rustflags wholesale (build.rustflags AND
# [target.'cfg(...)'].rustflags) — cargo does not merge them. To add
# -Cinstrument-coverage without silently dropping the repo's flags (e.g. -A warnings,
# and mold if it ever returns), we reconstruct those flags from .cargo/config.toml and
# prepend them. Override with COVER_BASE_RUSTFLAGS='...' (empty string = keep none).
cover_guard_rustflags() {
  if [ -n "${COVER_BASE_RUSTFLAGS+x}" ]; then return; fi
  local cfg="$REPO_ROOT/.cargo/config.toml"
  if [ ! -f "$cfg" ]; then COVER_BASE_RUSTFLAGS=""; return; fi
  # Pull every quoted token out of every `rustflags = [ ... ]` array (comments are
  # unquoted, so extracting quoted strings ignores them). Needs node (present in-repo).
  COVER_BASE_RUSTFLAGS="$(node -e '
    const fs = require("fs");
    const s = fs.readFileSync(process.argv[1], "utf8");
    const arr = /rustflags\s*=\s*\[([\s\S]*?)\]/g, tok = /"((?:[^"\\]|\\.)*)"/g;
    const out = []; let m;
    // strip line comments first, else a quoted string inside a # comment is picked up
    while ((m = arr.exec(s))) { let t = m[1].replace(/#[^\n]*/g, ''), x; while ((x = tok.exec(t))) out.push(x[1]); }
    process.stdout.write(out.join(" "));
  ' "$cfg" 2>/dev/null || true)"
  if [ -n "$COVER_BASE_RUSTFLAGS" ]; then cover_log "preserving config rustflags: $COVER_BASE_RUSTFLAGS"; fi
  return 0
}

# Three separate steps, so the empty report keeps its integrity:
#   baseline (EMPTY) : build instrumented per-crate harnesses, run NONE -> report/baseline.lcov
#                      (every member at 0%). lcov.info := baseline.lcov.
#   test  (TESTS+MERGE): ensure a baseline exists, run the tests under cover/<proj>/test/
#                      -> report/tests.lcov, then MERGE baseline.lcov + tests.lcov (at the
#                      lcov level, by file/line/function) -> report/lcov.info.
# The empty baseline and the test run are always distinct artifacts; the final report is a
# post-hoc merge — a test run can never mutate or contaminate the empty baseline.
cover_run() {
  local proj="$1" mode="${2:-baseline}"
  case "$mode" in baseline|test) ;; *) cover_die "mode must be 'baseline' or 'test', got '$mode'";; esac
  cover_check_tools
  local pdir="$COVER_ROOT/$proj"
  [ -d "$pdir" ] || cover_die "no such project dir: cover/$proj"
  local work="$pdir/.work" report="$pdir/report"
  mkdir -p "$work" "$report"

  cover_setup_env "$proj" "$work"

  if [ "$mode" = "baseline" ]; then
    rm -f "$work/owned-prefixes.txt"          # recompute ownership for the authoritative empty step
    cover_baseline_step "$proj" "$work" "$report"
    cp "$report/baseline.lcov" "$report/lcov.info"
    cover_summary "$proj" "empty (baseline, no tests run)" "$report/lcov.info" > "$report/summary.txt"
    cat "$report/summary.txt" >&2
    cover_log "[$proj] wrote report/baseline.lcov + report/lcov.info (empty 0% report)"
  else
    if [ ! -s "$report/baseline.lcov" ] || [ -n "${COVER_FORCE_BASELINE:-}" ]; then
      cover_log "[$proj] no baseline.lcov (or COVER_FORCE_BASELINE set) -> building the empty baseline first"
      cover_baseline_step "$proj" "$work" "$report"
    else
      cover_log "[$proj] reusing existing report/baseline.lcov as the empty base"
    fi
    cover_tests_step "$proj" "$work" "$report"
    cover_merge_step "$proj" "$report"
  fi
}

# Shared env + resource caps for every cargo invocation. Sets globals: COVER_NICEP[].
cover_setup_env() {
  local proj="$1" work="$2"
  cover_guard_rustflags
  # ONE target dir shared by every project: the report is filtered by owned-prefix after
  # export, so sharing compiled artifacts is safe and the heavy vendored deps
  # (bevy/wgpu/smithay) + shared support.* crates compile once, not per project.
  export CARGO_TARGET_DIR="${COVER_TARGET_DIR:-$COVER_ROOT/.cache/target}"
  export CARGO_INCREMENTAL=0
  # Coverage needs line tables, not full debug symbols -> smaller target, faster links.
  export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-line-tables-only}"
  # --- Resource safety (see ../../RESOURCE.md): no systemd/cgroups + zram swap here, so
  # the only freeze guard is keeping peak RAM under physical memory. Cap concurrency
  # (default 4, ~18 GiB peak in a 20 GiB budget) and shrink ld.bfd's per-link peak.
  export CARGO_BUILD_JOBS="${COVER_JOBS:-4}"
  local low_mem_link="-Clink-arg=-Wl,--no-keep-memory -Clink-arg=-Wl,--reduce-memory-overheads"
  export RUSTFLAGS="${COVER_BASE_RUSTFLAGS:+$COVER_BASE_RUSTFLAGS }-Cinstrument-coverage $low_mem_link"
  COVER_NICEP=()
  command -v nice   >/dev/null 2>&1 && COVER_NICEP+=(nice -n "${COVER_NICE:-15}")
  command -v ionice >/dev/null 2>&1 && COVER_NICEP+=(ionice -c3)
  cover_log "[$proj] resource caps: CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS, low-mem BFD link, ${COVER_NICEP[*]:-no nice/ionice}"
}

# Owned source prefixes for a project = manifest dirs of every member of its workspace
# roots (cargo metadata --no-deps -> a file is attributed to exactly one project). Cached.
cover_ensure_owned() {
  local proj="$1" work="$2" ownedf="$work/owned-prefixes.txt"
  [ -s "$ownedf" ] && return 0
  local roots root abs; roots="$(cover_roots "$proj")"
  : > "$ownedf"
  cover_log "[$proj] computing owned source prefixes (cargo metadata)"
  while IFS= read -r root; do
    [ -n "$root" ] || continue; abs="$REPO_ROOT/$root"; [ -d "$abs" ] || continue
    ( cd "$abs" && cargo metadata --no-deps --format-version 1 2>/dev/null ) \
      | jq -r '.packages[].manifest_path | rtrimstr("/Cargo.toml")' >> "$ownedf" 2>/dev/null || true
  done <<< "$roots"
  sort -u "$ownedf" -o "$ownedf"
}

# Resolve a fully-qualified crate (package) name -> its crate directory.
cover_crate_dir() {
  local fqn="$1" f
  f="$(grep -rl --include=Cargo.toml "^name = \"$fqn\"\$" "$REPO_ROOT"/compositor.* 2>/dev/null | head -1)"
  if [ -n "$f" ]; then dirname "$f"; fi
  return 0
}

# Build the instrumented per-crate test harnesses for every workspace root (this is what
# puts each crate's coverage map into a loadable executable). Sets globals: COVER_BINS[].
cover_build_workspaces() {
  local proj="$1" work="$2"
  local roots; roots="$(cover_order_roots "$(cover_roots "$proj")")"
  local skipf="$work/skipped.txt"; : > "$skipf"
  COVER_BINS=()
  local root abs exe tag bj berr why
  cover_log "[$proj] building instrumented harnesses for roots:"; printf '  %s\n' $roots >&2
  while IFS= read -r root; do
    [ -n "$root" ] || continue
    abs="$REPO_ROOT/$root"
    [ -d "$abs" ] || { cover_log "[$proj] SKIP $root (missing dir)"; echo "$root  (missing dir)" >> "$skipf"; continue; }
    tag="$(printf '%s' "$root" | tr '/.' '__')"; bj="$work/build-$tag.json"; berr="$work/build-$tag.err"
    cover_log "[$proj] build (instrumented): $root"
    if ! ( cd "$abs" && "${COVER_NICEP[@]}" cargo test --no-run --message-format=json ) > "$bj" 2>"$berr"; then
      why="$( { grep -m1 -iE 'not found|could ?n.?t find|unable to find|error: linking|error\[|panicked at' "$berr" 2>/dev/null || true; } | sed 's/^[[:space:]]*//' | cut -c1-120 )"
      cover_log "[$proj] SKIP $root (build failed: ${why:-see $berr})"
      echo "$root  (build failed: ${why:-see cover/$proj/.work/build-$tag.err})" >> "$skipf"; continue
    fi
    while IFS= read -r exe; do [ -n "$exe" ] && COVER_BINS+=("$exe"); done \
      < <(jq -r 'select(.executable != null) | .executable' "$bj")
  done <<< "$roots"
  local nskip; nskip="$(grep -c . "$skipf" 2>/dev/null || true)"; nskip="${nskip:-0}"
  [ "${#COVER_BINS[@]}" -gt 0 ] || cover_die "no instrumented executables produced for '$proj' ($nskip root(s) skipped — see $skipf)"
  if [ "$nskip" -gt 0 ]; then cover_log "[$proj] NOTE: skipped $nskip root(s) (listed in report/summary.txt)"; fi
  return 0
}

# cover_export <profraw_dir> <bins_array_name> <ownedf> <out_lcov> <work_prefix>
# merge profiles -> export lcov (arg-safe via file-list + @response-file) -> keep only
# records for files the project owns. Empty profraw set => empty lcov (not an error).
cover_export() {
  local pdir="$1"; local -n _bins="$2"; local ownedf="$3" out="$4" wp="$5"
  find "$pdir" -name '*.profraw' -type f > "$wp.profraw.list" 2>/dev/null || true
  if [ ! -s "$wp.profraw.list" ] || [ "${#_bins[@]}" -eq 0 ]; then : > "$out"; return 0; fi
  "$LLVM_PROFDATA" merge -sparse --input-files="$wp.profraw.list" -o "$wp.profdata"
  local e; : > "$wp.objects.args"
  for e in "${_bins[@]}"; do printf -- '-object\n%s\n' "$e"; done > "$wp.objects.args"
  ( cd "$REPO_ROOT" && "$LLVM_COV" export --format=lcov --instr-profile="$wp.profdata" \
      --ignore-filename-regex="$COVER_IGNORE_RE" "@$wp.objects.args" ) > "$wp.raw.lcov" 2>"$wp.llvm-cov.log" \
    || cover_die "llvm-cov export failed (see $wp.llvm-cov.log)"
  awk -v pf="$ownedf" '
    BEGIN { while ((getline p < pf) > 0) if (p != "") pre[++n] = p }
    /^SF:/ { f = substr($0, 4); keep = 0
             for (i = 1; i <= n; i++) if (index(f, pre[i] "/") == 1 || f == pre[i]) { keep = 1; break } }
    { buf = buf $0 "\n" }
    /^end_of_record/ { if (keep) printf "%s", buf; buf = ""; keep = 0 }
  ' "$wp.raw.lcov" > "$wp.owned.lcov"
  # llvm-cov emits a file once per object that contains it (a crate linked into many
  # harnesses appears many times). Dedupe via the merger (union by file/line) so counts
  # aren't inflated and the baseline denominator matches the merged report.
  node "$COVER_LIB_DIR/lcov-merge.js" "$wp.owned.lcov" > "$out"
}

# EMPTY step: instrumented harnesses run with --list (enumerate, execute nothing) -> a
# report where every member is present at 0%.
cover_baseline_step() {
  local proj="$1" work="$2" report="$3" exe
  cover_ensure_owned "$proj" "$work"
  rm -rf "$work/profraw"; mkdir -p "$work/profraw"
  export LLVM_PROFILE_FILE="$work/profraw/%p-%m.profraw"
  cover_build_workspaces "$proj" "$work"
  cover_log "[$proj] EMPTY step: running ${#COVER_BINS[@]} harness(es) with --list (no tests execute)"
  for exe in "${COVER_BINS[@]}"; do "$exe" --list >/dev/null 2>&1 || true; done
  cover_export "$work/profraw" COVER_BINS "$work/owned-prefixes.txt" "$report/baseline.lcov" "$work/base"
  cover_log "[$proj] wrote report/baseline.lcov"
}

# TESTS step: for each cover/<proj>/test/<fully-qualified-crate>/ dir, generate a tiny
# standalone package that depends on that crate by path and treats every *.rs in the dir
# as an integration test, then build+run it (instrumented) to record real coverage.
cover_tests_step() {
  local proj="$1" work="$2" report="$3"
  cover_ensure_owned "$proj" "$work"
  local tdir="$COVER_ROOT/$proj/test"
  rm -rf "$work/tprofraw" "$work/harness"; mkdir -p "$work/tprofraw" "$work/harness"
  export LLVM_PROFILE_FILE="$work/tprofraw/%p-%m.profraw"
  local -a tbins=()
  local fqndir fqn cratedir hp tbj e count=0 nf
  shopt -s nullglob
  for fqndir in "$tdir"/*/; do
    fqn="$(basename "$fqndir")"
    local rs=("$fqndir"*.rs); nf=${#rs[@]}
    [ "$nf" -gt 0 ] || continue
    cratedir="$(cover_crate_dir "$fqn")"
    if [ -z "$cratedir" ]; then
      cover_log "[$proj] test: no crate named '$fqn' in repo -> skipping that test dir"; continue
    fi
    hp="$work/harness/$fqn"; mkdir -p "$hp/tests"
    # Standalone package: depends on the target crate by path; empty [workspace] so it is
    # not swept into any real workspace. Every file in the test dir becomes a test target.
    {
      echo '[package]'
      echo "name = \"covtest_${fqn}\""
      echo 'version = "0.0.0"'
      echo 'edition = "2024"'
      echo 'publish = false'
      echo
      echo '[dependencies]'
      echo "${fqn} = { path = \"${cratedir}\" }"
      echo
      echo '[workspace]'
    } > "$hp/Cargo.toml"
    cp "$fqndir"*.rs "$hp/tests/"
    cover_log "[$proj] TESTS step: $fqn ($nf test file(s))"
    tbj="$work/thbuild-$(printf '%s' "$fqn" | tr '/.' '__').json"
    if ! ( cd "$hp" && "${COVER_NICEP[@]}" cargo test --no-run --message-format=json ) > "$tbj" 2>"$hp/build.err"; then
      cover_log "[$proj] test: build FAILED for $fqn (see cover/$proj/.work/harness/$fqn/build.err) -> skipping"; continue
    fi
    while IFS= read -r e; do [ -n "$e" ] && tbins+=("$e"); done \
      < <(jq -r 'select(.executable != null) | .executable' "$tbj")
    ( cd "$hp" && "${COVER_NICEP[@]}" cargo test --quiet ) >/dev/null 2>&1 || true   # run -> .profraw
    count=$((count + 1))
  done
  shopt -u nullglob
  if [ "$count" -eq 0 ] || [ "${#tbins[@]}" -eq 0 ]; then
    cover_log "[$proj] TESTS step: no runnable tests under cover/$proj/test/ -> empty tests.lcov"
    : > "$report/tests.lcov"; return 0
  fi
  cover_export "$work/tprofraw" tbins "$work/owned-prefixes.txt" "$report/tests.lcov" "$work/tests"
  cover_log "[$proj] TESTS step: ran $count crate test-set(s) -> report/tests.lcov"
}

# MERGE step: baseline.lcov (all members, 0%) + tests.lcov (executed) -> lcov.info, merged
# at the lcov level (by file/line/function name) so covmap-hash differences don't matter.
cover_merge_step() {
  local proj="$1" report="$2"
  local -a inputs=("$report/baseline.lcov")
  local label="baseline only (no tests)"
  if [ -s "$report/tests.lcov" ]; then inputs+=("$report/tests.lcov"); label="baseline + tests"; fi
  node "$COVER_LIB_DIR/lcov-merge.js" "${inputs[@]}" > "$report/lcov.info"
  cover_summary "$proj" "merged ($label)" "$report/lcov.info" > "$report/summary.txt"
  cat "$report/summary.txt" >&2
  cover_log "[$proj] merged $label -> report/lcov.info"
}

# Human-readable rollup computed straight from the lcov, so it always matches it.
cover_summary() {
  local proj="$1" mode="$2" lcov="$3"
  awk -v proj="$proj" -v mode="$mode" '
    /^SF:/ { files++ }
    /^FNF:/ { fnf += substr($0,5) }
    /^FNH:/ { fnh += substr($0,5) }
    /^LF:/  { lf  += substr($0,4) }
    /^LH:/  { lh  += substr($0,4) }
    END {
      lp = lf ? 100*lh/lf : 0
      fp = fnf ? 100*fnh/fnf : 0
      printf "project : %s\n", proj
      printf "mode    : %s\n", mode
      printf "files   : %d\n", files
      printf "functions: %d/%d covered (%.2f%%)\n", fnh, fnf, fp
      printf "lines    : %d/%d covered (%.2f%%)\n", lh, lf, lp
    }' "$lcov"
}

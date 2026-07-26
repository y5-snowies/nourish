# cover/ — tests & coverage

One **LLVM source-based coverage** report per project. This is the Rust analogue of
V8's precise ("byte"/region) coverage: rustc records a coverage *map* for every
function and region; running the instrumented binaries records execution *counts*;
`llvm-cov` joins them into `lcov`. With no tests, the map still enumerates every
member and the counts are zero — a valid lcov at **0% that captures all members**.

## Three separate steps (integrity of the empty report)

The empty baseline and the test run are always **distinct artifacts**; the final report
is a post-hoc merge, so a test run can never mutate or contaminate the empty baseline:

1. **EMPTY** — `generate.sh` builds instrumented harnesses, runs **no** tests, and writes
   `report/baseline.lcov`: every member at 0%. Generated independently → provably empty.
2. **TESTS** — `test.sh` runs the tests under `test/<crate>/` and writes
   `report/tests.lcov`: the executed coverage.
3. **MERGE** — `test.sh` then merges `baseline.lcov + tests.lcov` (at the lcov level, by
   file/line/function) into `report/lcov.info` — the final report. Because the merge is
   line-keyed, it's robust to the per-build hash in rustc's mangled names, and the
   denominator stays equal to the baseline's (tests only flip members from 0 → hit).

Tests are **not** kept in shipping source files — they live under `cover/<project>/test/`
(see "Writing tests" below).

## Layout

```
cover/
  projects.json          project -> workspace-root(s) mapping
  lib/cover.sh           all coverage logic (sourced by the scripts)
  lib/lcov-merge.js      lcov-level merge/dedupe (baseline + tests -> final)
  lib/report.js          render the lcovs into one HTML page
  report.sh              generate cover/report.html (a viewer for all projects)
  <project>/
    report/              generated (git-ignored):
                           baseline.lcov  empty 0% report (integrity artifact)
                           tests.lcov     executed coverage from the tests
                           lcov.info      final = baseline (+) tests
                           summary.txt    files / functions / lines rollup
    script/generate.sh   EMPTY step  -> baseline.lcov (+ lcov.info)
    script/test.sh       TESTS+MERGE -> tests.lcov, then lcov.info
    test/<crate>/*.rs    the tests, one dir per fully-qualified crate under test
```

## Projects

| project      | workspace root(s)                                             |
|--------------|---------------------------------------------------------------|
| `compositor` | every Cargo workspace root **except** installer & model (dynamic — includes orchestration, support.\*, expansion.\*, extension.\*, kernel.\*, introspection) |
| `installer`  | `compositor.installer/installer.process`                      |
| `model`      | `compositor.model`                                            |

A source file is attributed to the project whose workspace **owns** it
(`cargo metadata --no-deps`). Cross-workspace path deps stay with their owner — e.g.
every compositor root links `compositor.model`, but the model crates count under
`model`, not `compositor`.

## Usage

```bash
cover/<project>/script/generate.sh    # EMPTY: 0% baseline, all members captured
cover/<project>/script/test.sh        # TESTS+MERGE: run tests, merge onto baseline
cover/report.sh                       # render cover/report.html (all projects)
```

`lcov.info` is filtered to files this project owns and feeds any lcov consumer
(`genhtml`, Codecov, editor gutters, `llvm-cov`, or `cover/report.html`).

## Writing tests

Add a directory named for the **fully-qualified crate** you want to cover, and drop one or
more `*.rs` files in it — each file is a Cargo integration test of that crate:

```
cover/<project>/test/<fully_qualified_crate_name>/<any_name>.rs
```

```rust
// cover/dev/test/compositor_model_stats_registry_hdr/smoke.rs
use compositor_model_stats_registry_hdr::hdr_tuning;

#[test]
fn calls_hdr_tuning() { let _ = hdr_tuning(); }
```

The directory name maps straight to the crate (flat, never nested), so tests run against
exactly that crate deterministically. `test.sh` generates a tiny standalone package per
directory that depends on the crate by path, compiles every file as an integration test,
runs them under instrumentation, and merges the result onto the empty baseline.

## Requirements

- stable `rust` / `cargo`
- `llvm-profdata` + `llvm-cov` **matching rustc's LLVM version** (`rustc -vV | grep LLVM`;
  Fedora: `sudo dnf install llvm`). Override discovery with `LLVM_PROFDATA=` / `LLVM_COV=`.
- `jq`

## Notes

- The instrumented build is isolated in a shared `cover/.cache/target` (one target dir
  for all projects, so heavy vendored deps compile once) and never touches the normal
  `target/`. `cover/*/.work/` holds per-project profraw/intermediate lcov. Both are
  git-ignored and safe to delete (removing `.cache` forces a full recompile next run).
- `RUSTFLAGS` is set to `-Cinstrument-coverage` (plus low-memory BFD link flags) for the
  coverage build only. Any `build.rustflags` in a repo `.cargo/config.toml` (e.g. `-A
  warnings`) are auto-detected and **preserved** — instrumentation is added on top, never
  dropping them. Override with `COVER_BASE_RUSTFLAGS='<flags>'`.
- Resource caps (`COVER_JOBS`, low-mem links) keep the build inside a memory budget so it
  can't thrash the host — see `../RESOURCE.md`.
- Adding/removing crates needs no edit here: roots are discovered and members come
  from `cargo metadata`. Only `projects.json` changes if a whole new *project* is added.

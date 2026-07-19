# installer — tests

Tests are **not** defined inside shipping source files. They live here, organized by the
**fully-qualified crate name** they target:

```
cover/installer/test/<fully_qualified_crate_name>/<any_name>.rs
```

- The **directory name is the crate under test** (its package name, e.g.
  `compositor_installer_process_layout_compute_session`). This is flat — one directory
  per crate, never nested — and deterministic: the harness maps the name straight to the
  crate and runs the tests against exactly it.
- **Every `*.rs` file in that directory is a test** (a Cargo integration test). It may
  `use <fully_qualified_crate_name>::...;` and call the crate's public API.

`../script/test.sh` builds a tiny standalone package per directory that depends on the
target crate, treats each file as an integration test, runs them under instrumentation,
and merges the result onto the empty baseline. Until a directory exists here, the crate
stays at 0% in the report.

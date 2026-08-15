---
name: build
description: Build, check, run, or install the y5_compositor binary. Use whenever you need to compile the compositor, confirm the tree still compiles, run it, or deploy it. NEVER run `cargo build`/`cargo check` directly — the repo is 30 independent workspaces and a bare cargo builds a full multi-GB dependency tree per workspace root. Always use the environment/ scripts.
---

# build

## The whole surface

| Intent | Command |
|---|---|
| does it compile? | `environment/check.sh [workspace-dir]` |
| build the binary | `environment/build.sh [winit\|udev]` |
| build + install | `environment/build.sh udev --deploy` |
| run it nested | `environment/run-host.sh [winit\|udev]` |
| shipped artifact | `environment/build-optimized.sh udev` |

Every one of these prints the built binary path on stdout and its logs on stderr, so
`BIN="$(environment/build.sh udev)"` works.

## Cargo.toml is generated — never edit one

Every `Cargo.toml` and `Cargo.lock` in the linked tree is a build artifact: gitignored,
and rewritten by `workspace.generate.js` on every build. A fresh clone has none of them
until a build or check runs, so **rust-analyzer / Zed will not load a fresh clone** until
you run `environment/check.sh` (or `compositor.workspace/link.all.sh`, which just generates + lints).

Dependencies are declared in `crate.json` beside `lib.rs` — names only. Versions and
features live in `compositor.workspace/vendor.catalog.json`; per-crate features, `[[bin]]`,
build scripts and publish metadata live in `compositor.workspace/workspace.catalog.json`.
See CLAUDE.md → "Cargo manifests are GENERATED".

## Never run cargo directly

The repo is **30 independent top-level workspaces**. Cargo's default is a `target/`
at each workspace root, so a bare `cargo check`/`build`/`test` run per workspace
builds a complete dependency tree **per root** — that is how a checkout reaches
100+ GB. `.cargo/config.toml` now pins `[build] target-dir` to one shared directory
so a stray cargo can no longer scatter trees, but the scripts remain the interface:
they also run the workspace-conformance lint gate and pick the right profile.

There is no "quick scoped cargo check" exception. If you want to check one workspace,
that is what `environment/check.sh <workspace-dir>` is for.

## Profiles: there is one

Local work is **always `release-fast`** — release optimizations without LTO. It is not
selectable; `build.sh`, `run-host.sh` and `check.sh` take no profile argument and
reject one if given.

- **`debug` no longer exists anywhere.** Its dependency tree cost ~4× the release-fast
  one (measured: 14 GB vs 3.6 GB). `release-fast` inherits `debug = "line-tables-only"`,
  so backtraces still carry line numbers.
- **fat-LTO `release` is `build-optimized.sh` and nothing else.** Its final link is
  serial and takes 10–20 minutes. It belongs to release tags and published artifacts.
  Do not reach for it to "check that it compiles" — that is `check.sh`.

`[profile.release-fast]` is declared in **every** workspace root, because cargo only
honours `[profile.*]` at the root it is invoked from.

## check.sh

```bash
environment/check.sh                              # lint gate + cargo check, every root
environment/check.sh compositor.orchestration     # just that root
```

It exports `CARGO_TARGET_DIR` to the shared tree and checks with `--profile
release-fast`, the same profile `build.sh` builds with — so a check straight after a
build reuses the artifacts already on disk instead of growing a second tree.

## build.sh

```bash
environment/build.sh                       # winit (nested), release-fast
environment/build.sh udev                  # DRM/KMS on real hardware / a TTY
environment/build.sh udev --deploy         # ... + install to /usr/bin/y5.compositor
environment/build.sh udev --deploy=y5.compositor.exp   # ... to /usr/bin/y5.compositor.exp
```

`--deploy` copies (it does not move — a move forces a full relink next build) and
re-applies `cap_sys_nice+ep`, which the copy drops.

The backend is a compile-time cargo feature: `winit` is the default (`backend-winit`),
`udev`/`native` build `--no-default-features --features backend-native`.

## Notes

- Do **not** set `RUSTFLAGS` — it replaces `.cargo/config.toml`'s rustflags wholesale
  and silently drops `-A warnings` and `-C target-cpu=x86-64-v3`.
- That `target-cpu=x86-64-v3` means binaries **SIGILL on pre-Haswell CPUs**. Narrow the
  `[target.'cfg(target_arch = "x86_64")']` block when building for generic hardware.
- `build.sh` and `check.sh` run `workspace.lint.js` first and fail on violations.
  `Y5_SKIP_LINT=1` skips it (the distro bundle images set this; CI lints separately).
- **`check.sh` additionally passes `--deps`**, the cargo-shear-backed `deps-unused`
  rule. It costs minutes, so `build.sh` does not — and neither does GitHub CI, which
  goes through `build-optimized.sh` → `build.sh`. When `--deps` IS passed, a missing
  `cargo-shear` is a hard failure rather than a skip (`environment/install-deps.sh`
  installs it): asking for the rule and silently not getting it is worse than not
  asking, because the run goes green having checked nothing.
- **SHELL rule:** `lib.rs` / `mod.rs` may contain only `mod` declarations, `use`/`pub use`,
  `extern crate`, `#![…]` and `instance!()` — never a `fn`, `struct`, `impl`, `const` or
  `macro_rules!`. Put definitions in the crate's module file and re-export them. This one
  is **not allowlistable**; see CLAUDE.md → "SHELL".
- `Y5_TARGET_DIR` redirects the build's target dir. Only set it when a build must NOT
  share the repo tree — the containers (`/y5-target`) and the cross compiler do.
- Running the compositor needs a Wayland/DRM session.
- See `environment/README.md` for the container loop and the cross compiler.

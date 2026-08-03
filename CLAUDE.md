# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

It is written to describe **conventions and discovery commands**, not fixed file lists or
directory names, so it stays correct as crates and workspaces are added or renamed. When you
need a concrete name or path, run the discovery command shown rather than trusting a hard-coded
value.

## What this is

`y5` is a Wayland compositor written in Rust. It is built on **vendored** forks of
`smithay` (Wayland), `bevy` + `wgpu` + `naga_oil` (rendering) and `iced` + `cryoglyph`
(UI), all under `vendor/` and patched in-tree.

The shipped binary is **`y5_compositor`** — the crate that declares it via `[[bin]]` is the
compositor's entry point. Locate it without assuming the path:

```bash
# crate dir that produces the y5_compositor binary
dirname "$(grep -rl --include=Cargo.toml 'name = "y5_compositor"' compositor*)"
```

Toolchain is **stable** (`rust-toolchain.toml` at the repo root); crates use Rust edition 2024
(stable since 1.85), so a current stable toolchain builds the tree. There are no nightly-only
language features in use.

## Multi-workspace architecture (read this first)

The repo is NOT a single Cargo workspace. It is a set of **independent top-level Cargo
workspaces** that share crates by path. Each workspace is a top-level `compositor*/` directory:

```bash
ls -d compositor*/        # the current set of workspaces
```

Top-level `compositor*` dirs are either workspace roots or **containers** of workspace
roots (see `document/ARCHITECTURE.md`): `compositor.orchestration/` (the driving layer),
`compositor.support/` (system core, smithay dispatch, shared libs),
`compositor.expansion/` (integral expansions: `compositor.y5` — the stock experience —
plus `compositor.remote`, `compositor.background`), `compositor.extension/` (add-only:
`compositor.monitor`), and `compositor.kernel/` (the hardware layer: one root per domain
— `kernel.vulkan`, `kernel.drm`, ... — plus `kernel.loader`, which holds the
`y5_compositor` `[[bin]]`). Don't rely on this list being exhaustive — `ls -d
compositor*/` is the source of truth.

`compositor.model/` is the **bottom** of that graph and belongs to no layer: the
shared shapes the compositor, the installer and the external developer tool all
read — `model.debug` (the logging macros), `model.environment` (every persisted
setting, i.e. what `preferences.json` holds), `model.log` (the gRPC log process)
and `model.stats`. It depends on no compositor crate, and nothing that is
compositor behaviour belongs in it — runtime state and policy go in the layer
that owns them, not here. `compositor.developer/` is the separate developer
project (the log viewer and the stress harnesses under `developer.tool/`); it is
outside the link graph entirely and **no compositor crate may depend on it**.

These workspaces depend on each other's crates via **generated** `[path]` dependencies, not by
being members of one workspace. The wiring is mechanical:

- `workspace.link.js` discovers **every** workspace root in the repo (any `compositor.*/`
  or `compositor.*/*/` dir whose `Cargo.toml` has a `[workspace]` members array, minus the
  standalone trees: the installer and `developer.tool`), and rewrites a
  `# --- GENERATED WORKSPACE LINKS START/END ---` block in the root `Cargo.toml` of the
  workspace it is invoked from, with a `crate = { path = "..." }` entry for every internal
  crate. Generation is GLOBAL — the per-dir `link.json` files are documentation of intent
  and are not read by the script.
- An optional `link.features.json` in a root overrides feature attributes for a generated
  entry (feature selection lives at the root, never in a crate).
- `./link.all.sh` runs `workspace.link.js` in each top-level workspace — a hand-maintained
  list, so a NEW workspace root must be added to it.

**Consequence:** after adding, removing, or renaming any crate or workspace, you MUST run
`./link.all.sh` from the repo root, or the cross-workspace path links go stale and downstream
workspaces fail to resolve the crate. Do not hand-edit the GENERATED block.

## Crate / directory naming convention

Crates live exactly two levels below a workspace root, with a strict **chain-prefix** naming
scheme enforced by the workspace member globs (e.g. `compositor.action/*/*`):

```
compositor.expansion/        container (organizational only for compositor.<x> roots)
  compositor.y5/             workspace root        (root_tail = "y5")
    y5.action/               L0  = <tail>.<seg>
      action.camera/         L1  = <seg>.<sub>
        camera.find/         L2  = the crate dir (<sub>.<name>)
```

The crate's package name is the dotted chain with dots → `_`. Roots named
`compositor.<x>` start the chain fresh (`compositor_y5_action_camera_find`); other roots
merge through their container (`compositor.support/support.smithay` →
`compositor_support_smithay_*`, `compositor.kernel/kernel.vulkan` →
`compositor_kernel_vulkan_*`). There is no `y5_` package prefix anywhere;
`workspace.lint.js` (run by `link.all.sh`, `environment/build.sh` and
`environment/check.sh`) enforces layout, chain, names, the 30~100-LOC single-module
size policy, FLAT crates — `lib.rs` plus at most one module file directly next to
`Cargo.toml`, never a `src/` dir — and DEPS: every dependency in a crate must be
`{name}.workspace = true` (paths + versions + feature selection live only at the
workspace root; no path/version/feature deps inside a crate), and WORLD-ID: rim
code must resolve the focused world via the Orchestrator focus accessors
(`camera()`/`canvas()`/`select()`/… — see `document/WORLD_DELEGATION.md`) or
`WorldManager::spawn_target()`, never a literal world id. The `MAIN_WORLD`
constant has been removed and the world-id allowlist in
`workspace.lint.allow.json` is empty (the rule is hard); standalone trees
(installer, dev-tools) are exempt from DEPS.

**To add a crate, use the `add-crate` skill** (`.claude/skills/add-crate/SKILL.md`), which
drives the `y5-template` binary and reminds you to run `link.all.sh`. Do not hand-create crate
dirs/`Cargo.toml`; the naming must match the convention exactly or the member globs won't pick
the crate up.

## Build & run

`cargo` commands must be run from inside a specific top-level workspace dir (there is no root
`Cargo.toml`). Standard `cargo build` / `cargo test` / `cargo test <name>` work per-workspace.

Compiler flags (warnings suppressed with `-A warnings`, and the **mold** linker) live in the
repo-root `.cargo/config.toml`, which every workspace inherits. **Do not set `RUSTFLAGS`** — it
replaces that config wholesale and silently drops the mold linker. See
`environment/README.md` → "Build speed".

Build just the driving layer or the y5 expansion:

```bash
cd compositor.orchestration && cargo build
cd compositor.expansion/compositor.y5 && cargo build
```

Build/run/deploy the actual compositor go through the scripts in **`environment/`** (host
build/run + release) and **`environment.container/`** (the containerized dev loop under nested
Wayland). See **`environment/README.md`** for the full list; the common ones are
`environment/run-host.sh [winit|udev] [debug|release]` and
`environment/build-release.sh <dev|system>` on the host, and
`environment.container/run.sh [debug|release]` + `environment.container/image.sh` for the
container loop. These scripts discover the entry crate and the workspace set themselves, so
they don't need editing when names change.

Running the compositor needs a Wayland/DRM session.

## Coordinate system

y5 has a non-obvious dual coordinate model (a pannable/zoomable "y5-world" vs. smithay storage
vs. render/physical space). The `Transform` type mediates all conversions. **Read
`document/TRANSFORM.md` before touching geometry/scaling/positioning code** — using the wrong
extraction mode (`into_storage_*()` vs `.into()`) silently produces wrong-space coordinates.

## Logging

y5 has its **own** tracing-free structured logging system. **All logging uses the macros from
`compositor_model_debug_instance_record` (`error!`/`warn!`/`info!`/`trace!`/`abort!`) — do
NOT use `tracing` or `log` in new/changed code.** Each crate declares its instance once in
`lib.rs` (the `add-crate` template does this automatically). **Read `document/LOGGING.md`** and
use the **`logging`** skill before adding or migrating log statements. Records stream to the
`compositor.developer/developer.tool` viewer over gRPC; levels are controlled by cargo features
(compile) and the `log_level` field in `settings.json` (read once at startup). The compositor
exports that value as the `COMPOSITOR_LOG_LEVEL` env var for child processes but does not read it
back to override the config.

## Reference docs

- **`document/TRANSFORM.md`** — the single canonical guide to y5's coordinate model: the three
  coordinate spaces, the `Transform` value and `Context`, the two extraction modes, worked
  examples for every usage pattern, off-thread/multi-thread integration notes, and common bugs.
  Read it whenever you touch positioning, scaling, rendering, input mapping, or damage.
- **`document/LOGGING.md`** — the structured logging system: the macros, per-crate `instance!()`,
  `abort!`, the feature/env level controls, the gRPC stream + viewer tool, and how to migrate
  `tracing` call sites. Read it whenever you add or change logging.

## Notes

- `vendor/*` are patched dependencies — changes there are intentional; treat them as part of
  the codebase, not as drop-in upstream.

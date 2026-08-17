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

## Build, check & run

**Never run `cargo build` / `cargo check` / `cargo test` directly.** The repo is 30
independent workspaces; cargo's default is a `target/` per workspace root, so a bare
cargo run in each builds a complete multi-GB dependency tree **per root**. That is how
a checkout reaches 100+ GB. Use the scripts:

| Intent | Command |
|---|---|
| does it compile? | `environment/check.sh [workspace-dir]` |
| build the binary | `environment/build.sh [winit\|udev]` |
| build + install | `environment/build.sh udev --deploy[=NAME]` |
| run it nested | `environment/run-host.sh [winit\|udev]` |
| shipped artifact | `environment/build-optimized.sh udev` |

Each prints the built binary's path on stdout, logs on stderr.

**There is one profile.** Local work is always `release-fast` (release optimizations,
no LTO); `build.sh`, `run-host.sh` and `check.sh` take no profile argument and reject
one. `debug` is gone — its dependency tree cost ~4x as much disk and `release-fast`
keeps `debug = "line-tables-only"`, so backtraces still have line numbers. Fat-LTO
`release` is `build-optimized.sh` alone: a 10-20 minute serial link, for release tags,
never for "does it compile".

`.cargo/config.toml` pins `[build] target-dir` to ONE tree for the whole repo (the
loader workspace's `target/`), relative to the config file so it survives worktrees.
`CARGO_TARGET_DIR` and `--target-dir` still override it, which is how the containers,
the cross compiler and the coverage runner keep their own.

Two things in `.cargo/config.toml` that will bite you:
- **Do not set `RUSTFLAGS`** — it replaces that config's rustflags wholesale, silently
  dropping `-A warnings` and `-C target-cpu=x86-64-v3`.
- That `target-cpu=x86-64-v3` (Haswell 2013+) applies to every profile, and the
  binaries **SIGILL on pre-v3 CPUs**. Narrow it when building for generic hardware.

Build/run/deploy details, the containerized dev loop and the cross compiler are in
**`environment/README.md`** and **`environment.container/`**. Running the compositor
needs a Wayland/DRM session.

## Multi-workspace architecture (read this first)

The repo is NOT a single Cargo workspace. It is a set of **independent top-level Cargo
workspaces** that share crates by path:

```bash
ls -d compositor*/                                              # the containers
grep -l '^\[workspace\]' compositor*/Cargo.toml compositor*/*/Cargo.toml   # the roots
```

Top-level `compositor*` dirs are either workspace roots or **containers** of workspace
roots: `compositor.orchestration/` (the driving layer), `compositor.support/` (system
core, smithay dispatch, shared libs), `compositor.expansion/` (integral expansions:
`compositor.y5` — the stock experience — plus `compositor.remote`, `compositor.background`,
`compositor.pipeline`), `compositor.extension/` (add-only: `compositor.monitor`,
`compositor.configurator`), and `compositor.kernel/` (the hardware layer: one root per
domain — `kernel.vulkan`, `kernel.drm`, ... — plus `kernel.loader`, which holds the
`y5_compositor` `[[bin]]`). Don't rely on this list being exhaustive — the commands
above are the source of truth.

`compositor.model/` is the **bottom** of that graph and belongs to no layer: the
shared shapes the compositor, the installer and the external developer tool all
read — `model.debug` (the logging macros), `model.environment` (every persisted
setting, i.e. what `preferences.json` holds), `model.log` (the gRPC log process)
and `model.stats`. It depends on no compositor crate, and nothing that is
compositor behaviour belongs in it — runtime state and policy go in the layer
that owns them, not here. `compositor.developer/` is the separate developer
project (the log viewer and the stress harnesses under `developer.tool/`); it is
outside the link graph entirely and **no compositor crate may depend on it**.

## Cargo manifests are GENERATED — never edit a Cargo.toml

**Every `Cargo.toml` and `Cargo.lock` in the linked tree is a build artifact.** They are
gitignored, and a fresh clone has none of them until the generator runs. Editing one is
pointless: the next build overwrites it.

Three authored sources replace them:

| file | what it holds |
|---|---|
| `vendor.catalog.json` | every EXTERNAL crate — exact version, feature presets, vendored path, per-workspace overrides, `[patch.crates-io]` |
| `workspace.catalog.json` | every root's members/resolver/link-features, plus each crate's non-dependency manifest facts (features, optional deps, `[[bin]]`, build script, publish metadata, version/edition exceptions) |
| `<crate>/crate.json` | that crate's dependency NAMES, and nothing else |

The first two live in **`compositor.workspace/`** — the folder that holds every piece of
workspace tooling — and are the overview: open them to see what the tree depends on and
how it is shaped. `crate.json` is machine data — 857 of them, each a list of
names. All three are **JSONC**: `//` comments are legal, and a comment above a dependency is
re-emitted above its line in the generated manifest.

`compositor.workspace/` holds all of it — the two catalogs, `workspace.generate.js`,
`workspace.lint.js` and its allowlist, `workspace.report.js`, the shared `jsonc.js` +
`rust.items.js` and `link.all.sh`. Nothing workspace-related is left loose at the repo root.

The generator writes every manifest and the `.gitignore` block that hides them. You
rarely run it by hand: `environment/build.sh` and `environment/check.sh` run it before
invoking cargo, which is what makes a fresh clone bootstrap itself and what retired the
old "you forgot to run `link.all.sh`" footgun — there is no committed generated block
left to go stale, so there is no drift to check.

| command | |
|---|---|
| `node compositor.workspace/workspace.generate.js --shape` | what does this tree depend on? Add a crate or root name for detail. |
| `… --check` | fail if any manifest is out of date |
| `… --clean` | delete every generated manifest + lock (only what it generates — never `vendor/`, the installer or the toolkit) |
| `compositor.workspace/link.all.sh` | regenerate + lint, to refresh an editor without building |

### Versions are EXACT pins

`vendor.catalog.json` writes registry versions as `"=1.2.3"`, and the generator
**refuses** anything else unless the entry says `"mode": "predicate"`. Cargo reads a
bare `"1.2.3"` as `^1.2.3`, which is how `zbus = "5.15.0"` was quietly building
against 5.19.0 — with the lockfiles gitignored, an unpinned direct dependency is
simply unreproducible. Vendored forks are exempt: their `path` is what cargo
resolves, and the version only mirrors the fork's own.

### Feature presets

A crate whose feature selection varies between workspaces declares **named presets**,
and each root selects one:

```jsonc
"presets": { "smithay": { "compositor": {...17 features...},
                          "hardware":   {...16, no renderer_pixman...},
                          "desktop":    { "features": ["desktop", "wayland_frontend"] } } },
"crates":  { "smithay": { "path": "vendor/smithay", "version": "0.7.0", "preset": "compositor" } },
"workspace": { "compositor.kernel":                 { "smithay": { "preset": "hardware"   } },
               "compositor.kernel/kernel.loader":   { "smithay": { "preset": "compositor" } } }
```

Resolution is base → container → exact root, and a preset REPLACES the selection
rather than merging, so an override is always the whole answer. That container layer
is why eleven kernel roots take one line with two named exceptions.

This exists because writing the same seventeen features out thirteen times is how
they came to "differ": three of smithay's four apparent feature sets turned out to be
the same set in a different ORDER. Feature lists are sets — the generator sorts them.

A preset nothing selects fails the lint, same as a catalog entry nothing depends on.

Two properties worth knowing:

- **Feature selection lives at the root, never in a crate.** A `crate.json` cannot request a
  feature, a version or a manifest shape; those are `workspace.catalog.json`'s job. The DEPS
  lint has always enforced this for versions and paths — it now covers everything.
- **A root declares only what its members actually use.** The external and internal
  `[workspace.dependencies]` entries are the union of the names its crates list, so a root
  cannot accumulate dependencies nothing needs. (The previous generator spliced the *global*
  crate pool into every root: ~23,500 dead entries.)

`node workspace.generate.js --check` fails if anything is out of date. (The one-off
`workspace.fidelity.js`/`workspace.extract.js` checkers that gated the migration off
hand-written manifests are gone — there is nothing left to compare against.)

**Consequence:** after adding, removing or renaming a crate, write/move its `crate.json` and
rebuild. There is nothing else to remember, and no list to hand-maintain.

## Crate / directory naming convention

Crates live exactly two levels below a workspace root, with a strict **chain-prefix** naming
scheme enforced by the workspace member globs (e.g. `y5.camera/*/*`):

```
compositor.expansion/        container (organizational only for compositor.<x> roots)
  compositor.y5/             workspace root        (root_tail = "y5")
    y5.camera/               L0  = <tail>.<seg>
      camera.transform/      L1  = <seg>.<sub>
        transform.translate/ L2  = the crate dir (<sub>.<name>)
```

The crate's package name is the dotted chain with dots → `_`
(`compositor_y5_camera_transform_translate`). Roots named `compositor.<x>` start the
chain fresh; other roots merge through their container (`compositor.support/support.smithay`
→ `compositor_support_smithay_*`, `compositor.kernel/kernel.vulkan` →
`compositor_kernel_vulkan_*`). There is no `y5_` package prefix anywhere.

`workspace.lint.js` (run by `compositor.workspace/link.all.sh`, `environment/build.sh` and
`environment/check.sh`) enforces layout, chain, names, the 30~100-LOC single-module
size policy, FLAT crates — `lib.rs` plus module files directly next to `Cargo.toml`,
never a `src/` dir — SHELL (below), and DEPS: every dependency in a crate must be
`{name}.workspace = true` (paths + versions + feature selection live only at the
workspace root; no path/version/feature deps inside a crate). Standalone trees
(installer, dev-tools) are exempt from DEPS. Violations fail the build; `[advice:*]`
and `[allow:*]` lines are warnings, and `workspace.lint.allow.json` records every
per-crate exemption — both the plain `["size"]` array form and the structural
`{ "allow": [...], "reason": "..." }` form that used to live in the crate's own
`[package.metadata.lint]` (a generated manifest is no place for a standing exemption).

**SHELL:** a `lib.rs` or `mod.rs` **declares structure and re-exports it; it never
defines anything.** Permitted: `#![…]` inner attributes, `extern crate` (incl.
`#[macro_use]`), `mod`/`pub mod` declarations, `use`/`pub use`, `instance!()`, and an
inline `mod x { … }` whose body satisfies the same rule (a re-export grouping). A `fn`,
`struct`, `impl`, `const`, `static`, `trait`, type alias or `macro_rules!` is a failure —
move it into the crate's module file and re-export it, which keeps every existing
`<crate>::Item` path working.

The point is that a review can **skip these files**: if a shell file can only re-export,
its diff is the crate's API surface and nothing else. That guarantee is what makes the
rule **not allowlistable** — a single exemption puts the burden back on the reader, who
would have to check whether *this* `lib.rs` is one of the exempt ones before trusting it.
The whole tree conforms today (`workspace.lint.js` → 0 failures), so there is nothing to
grandfather. If a case ever genuinely cannot be worked around, re-introducing the
exemption path is a deliberate decision, not a line added to an allowlist.

The size policy does not count `lib.rs`/`mod.rs` LOC, since this rule makes every line in
them boilerplate the linter itself mandates.

The rule is backed by `compositor.workspace/rust.items.js`, a comment- and
string-aware top-level item scanner (a regex over lines gets this wrong in both
directions — a `//` inside a string, a brace inside a doc comment, a `use` in a
function body).

**WORLD-ID:** rim code must resolve the *focused* world via the Orchestrator focus
accessors (`camera()` / `canvas()` / `select()` / `pointer()` / … in
`compositor.orchestration/orchestration.core/core.state/state.base/state.rs`, each
marked `FOCUS ACCESSOR`) or `WorldManager::spawn_target()` — never a literal world id.
The rule is hard: `workspace.lint.allow.json` grants zero `world-id` exemptions. The
fixed ids themselves still exist (`MAIN_WORLD`, `LOCK_WORLD`, `PICKER_WORLD` in
`compositor.support/support.system/system.world/world.identity/identity.base`) and the
loader uses them to *construct* the static worlds at startup; what is banned is
reaching for one to answer "which world is focused".

**To add a crate, use the `add-crate` skill** (`.claude/skills/add-crate/SKILL.md`), which
drives the `y5-template` binary. Do not hand-create crate dirs; the naming must match the
chain convention exactly or the member globs won't pick the crate up — and the generator
derives the package name from that chain, so a wrong directory name is a hard error rather
than a silently odd crate.

## Coordinate system

y5 has a non-obvious dual coordinate model — a pannable/zoomable, **centre-anchored**
y5-world vs. smithay storage vs. render/physical space. The `Transform` type mediates
every conversion.

**Read the module docs at the top of
`compositor.expansion/compositor.y5/y5.camera/camera.transform/transform.translate/transform.rs`
before touching geometry, scaling, positioning, input mapping or damage.** They define
the three spaces and the two extraction modes. The trap they exist for: y5-world and
screen-logical are BOTH typed `Logical`, so using the wrong extraction mode
(`into_storage_*()` vs `.into()`) silently produces wrong-space coordinates rather than
a type error.

Two known mislabels to keep in mind: `PointerState::motion` is typed `Logical` but
holds **physical** pixels, and `Camera::position_previous` sits beside a `Logical`-typed
`transform.position` while holding physical too.

## Logging

y5 has its **own** tracing-free structured logging system. **All logging uses the macros from
`compositor_model_debug_instance_record` (`error!`/`warn!`/`info!`/`trace!`/`abort!`) — do
NOT use `tracing` or `log` in new/changed code.** Each crate declares its instance once in
`lib.rs` (the `add-crate` template does this automatically). Use the **`logging`** skill
before adding or migrating log statements. Records stream to the
`compositor.developer/developer.tool` viewer over gRPC; levels are controlled by cargo features
(compile) and the `log_level` field in `settings.json` (read once at startup). The compositor
exports that value as the `COMPOSITOR_LOG_LEVEL` env var for child processes but does not read it
back to override the config.

## Skills

`.claude/skills/` — `build` (build/check/run/deploy), `add-crate` (scaffold a crate via
`y5-template`), `logging` (the macros and `instance!()`). Prefer them over improvising.

## Notes

- `vendor/*` are patched dependencies — changes there are intentional; treat them as part of
  the codebase, not as drop-in upstream.
- The compositor reads **all** its configuration from `~/.config/y5.compositor/settings.json`.
  Every field is required and the process panics if one is missing — there are no defaults and
  no env-var configuration. See `environment/README.md`.

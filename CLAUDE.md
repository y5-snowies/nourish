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

## XWayland

X11 clients run on y5's **own** Xwayland server — there is no `xwayland-satellite`
proxy any more. The compositor spawns the `Xwayland` binary and is that server's X11
window manager in-process (smithay's `xwayland` feature). Three consequences shape the
code:

- **`XwmHandler` is implemented on `Dispatch`, not on `Loop`.** That is forced, not
  chosen: smithay's surface-association hook (`XWaylandShellState::new::<D>`) is a
  wayland PRE-COMMIT hook, so its `D` is the wayland dispatch state. It fits the
  existing split anyway — the X11 handlers are world-free and record onto outboxes
  (`Dispatch::xwayland`) that `Wire::drain_protocol` applies against the Space, exactly
  like every other protocol handler. Those outboxes are the SHARED ones —
  `new_toplevels`, `destroyed_toplevels`, `fullscreen_requests`, tagged with
  `find::Shell` — never X11-only lists, and there is exactly **one drain point**:
  `event_loop.run`'s per-iteration callback in the loader, after every source has
  dispatched. Wayland and X11 arrive on two different calloop sources, so draining per
  source would make the order a frame's events are applied in a function of which fd
  calloop polled first. Sources only set `Dispatch::protocol_pending`; the loop drains
  if it is set, which also keeps the drain off vblank-only wakes — its tail
  (`foreign_reconcile`, the geometry mirror) allocates per window. The X11 event source
  lives on a NESTED calloop loop (data = `Dispatch`) that the outer loop polls and
  pumps; see `compositor.kernel/kernel.loader/…/execute.base/xwayland.rs`. That pump is
  also what NOTICES the server dying: smithay's `XWayland` source reports only a
  *startup* failure (it disables itself after `Ready`), so `xwayland::died` — reached on
  a nested-loop dispatch error or the Xwayland wayland client vanishing — retires the
  X11 windows through the ordinary `destroyed_toplevels` path, drops the `X11Wm`,
  retracts `DISPLAY` and removes the pump. It does not respawn.
- **`smithay::desktop::Window` now has two shapes.** `window.toplevel()` is `None` for
  an X11 window, so anything above the wire layer goes through the facade crates rather
  than branching on its own: **`state.window/window.shell`** for talking to a window
  (`stage`/`send`/`send_pending`/`close`/`set_fullscreen`/`set_suspended`/
  `configured_size`/`pending_size`/`has_parent`), **`state.window/window.ident`** for
  who it belongs to (`pid`/`credentials`/`names`/`states`/`surface`), and
  **`state.window/window.find`** for keying a window by surface. The rule is
  mechanical, not a matter of judgement — **no `.toplevel()` and no `x11_surface()`
  outside `state.window/*`**, and two greps hold it:

  ```bash
  grep -rn "\.toplevel()" --include=*.rs compositor.* | grep -v state.window/window.
  grep -rn "x11_surface()" --include=*.rs compositor.expansion compositor.orchestration compositor.introspection
  ```

  Note the two surface accessors are NOT interchangeable: `ident::surface` is the
  window's surface for protocol-agnostic work (an X11 window has one; `None` only
  means Xwayland has not associated it yet), while `ident::xdg_surface` is the surface
  an xdg-only protocol lives on (`None` for X11 by design). Picking the wrong one is
  how a guard silently widens.
- **An X11 window is configured by ONE call carrying position and size together**, so
  `shell::stage` records the size and puts the window on a dirty list that ONE per-frame
  pass (`Orchestrator::flush_x11_configures`, run from `present.callbacks::housekeeping`)
  drains through `shell::take_staged` — nothing walks the Space for it, and a frame with
  nothing staged costs nothing. Callers never supply a position, and neither does the
  flush: **a configure only ever RESIZES**. A toplevel is always told `shell::X11_ORIGIN`;
  a child (`WM_TRANSIENT_FOR` or override-redirect) keeps the position its client chose,
  which is why `XwmHandler::configure_request` honours x/y for a transient and refuses
  them for a toplevel. `flush_pending` diffs against smithay's own `last_configure`, the
  rect the server was last given by ANY route (the client's granted ConfigureRequest
  included) — a private "last sent" cell went stale on exactly that route.

  **An X11 unmap is a HIDE, not a destroy.** `unmapped_window` keeps the `Window` in its
  Space with no `wl_surface` — not drawn, not hit — exactly as an xdg toplevel that
  commits a null buffer keeps its element; a remap resolves to the same element by
  identity in the drain, and only `DestroyNotify` (or the X server dying) retires it.
  Treating the unmap as a destroy made every Wine/SDL fullscreen toggle and GTK
  hide()/show() leave a placeholder and remap camera-centred under a new uuid.

  **y5 never moves an X11 window across X space.** X keeps its own layout: every
  toplevel at the origin, every child where its client put it.

  There is no honest position to send. A `Space` location is centre-anchored, unbounded
  y5-world; the X screen is corner-anchored and output-sized. Both attempts failed on
  hardware: world coordinates put every window off the X screen (pointer events clamp to
  an edge — X11 apps typed but could not be clicked), and projecting to SCREEN churned on
  every pan and sent the client's own child placements back in a space the compositor
  reads as world.

  **So the X STACK decides which client an ungrabbed pointer event reaches.** Xwayland
  turns a `wl_pointer` event into "window position + surface-local" and the X server
  hit-tests its own tree at the result; where X windows overlap — and they do, since X's
  layout owes nothing to the canvas — stacking is the answer. It is the ONLY lever:
  `XSendEvent` is flagged synthetic and toolkits ignore it, XTEST re-enters the same test.
  So:

  - `Dispatch::raise_x11_for_pointer` raises the window the pointer is entering, called
    from the rim **before** `PointerHandle::motion` (and from touch `down`). The ordering
    is load-bearing: raising after the enter leaves the first events of a crossing
    hit-tested against the old stack. The raise is WRITTEN AND FLUSHED, never waited for:
    a checked round trip parks the compositor's one thread on the X server, which a
    foreign `XGrabServer` or an Xwayland roundtrip on y5's own socket turns into a stall
    or a deadlock (`X11Wm::raise_window` explains).
  - `Dispatch::lower_new_x11` maps every new window at the BOTTOM, so it cannot take
    events from whatever the pointer is already on before the pointer reaches it.
  - **Nothing may publish a competing order.** `Wire::sync_x11_stacking` used to push
    y5's canvas order every drain and undid the raise within a frame; it is gone. The X
    stack is not a mirror of the canvas.

  This is xwayland-satellite's mechanism, verified in its source: `raise_to_top` in the
  `wl_pointer.enter` handler (`server/event.rs`), `StackMode::Below` on `MapRequest`.
  satellite is one compositor for the whole Xwayland instance, not one per window.

  Rejected, so they are not re-derived: world coordinates; screen projection; parking all
  but the pointer-focused window (a TRANSITION, so windows the pointer never visited kept
  the client's position — events over Firefox landed in xterm); and invented
  non-overlapping SLOTS, which worked but cannot scale — X window coordinates are `INT16`,
  so that space ends at 32767 a side while the canvas does not end at all, and a
  full-width window cut it in half after two clients.

  **What crosses instead is a DIFFERENCE**, and it is the recorded intent — never a live
  read of two slots, which would be an artefact of the allocation rather than anything the
  client said. `X11Popup` takes its offset as a constructor argument for the same reason.
  That is how xwayland-satellite did it (`create_popup` feeds `child.x - parent.x` into an
  `xdg_positioner` and never the absolutes).

**Every xdg protocol y5 reads has an X11 route.** `ident::xdg_surface` returning
`None` means "not the xdg one", never "X11 cannot answer" — the equivalents are
`_NET_STARTUP_ID` (activation), `WM_WINDOW_ROLE` + `WM_CLASS` (session identity) and
`_NET_WM_ICON` (icon). If a new caller appears, find the X11 property before
concluding the feature does not apply.

Three more facts that are easy to get wrong:

- **Never compare `X11Surface` with `==`.** Its `PartialEq` is
  `xwm == xwm && window == window && self_alive && other_alive` — a dead surface equals
  nothing, not even itself — and every teardown path holds a surface smithay has
  already marked dead. Compare `window_id()`. `find::by_x11` does.

- Every X11 window's `wl_surface` belongs to the **one** Xwayland client, so surface
  credentials name the X SERVER, not the app. The pid comes from `_NET_WM_PID` via
  `ident::pid` — which is what keeps the kill paths in `select.overlay` from taking
  down the X server, and what makes the Steam/tearing attribution in
  `graphic.tearing` work for X11 games at all.
- **An X11 window can declare itself ephemeral**, and `ident::is_ephemeral_x11` is that
  test: `_NET_WM_WINDOW_TYPE` (menu, tooltip, notification, splash, dnd, dock, …),
  `_NET_WM_STATE_MODAL`, or override-redirect. It is the X11 counterpart to the
  `xdg_wm_dialog_v1` hint, and it feeds the same ephemeral mark, so such a window
  leaves no placeholder. Both that mark and `DiscardPlaceholder` are written to the
  WINDOW as well as the surface: smithay clears an X11 window's surface association
  inside `unmapped_window`, the same event that queues its destroy, so a mark left only
  on the surface is unreadable by the time the drain applies it.
- **An EPHEMERAL X11 window that names a parent is a POPUP, not a window.**
  `window.child::as_popup` decides — `ident::is_ephemeral_x11` (override-redirect,
  `_NET_WM_STATE_MODAL`, or a menu/tooltip/dnd/splash `_NET_WM_WINDOW_TYPE`) plus a
  resolvable `WM_TRANSIENT_FOR` — and the drain tracks it in the `PopupManager` instead
  of mapping it: no uuid, no Space slot, no decoration, no placeholder, nothing in the
  dock or navigator. Draw and hit reach it through the paths that already walk
  `popups_for_surface`.

  The vendored `PopupKind::X11(X11Popup)` carries BOTH surfaces and both `X11Surface`s,
  because its `location()` is `child.last_configure().loc - parent.last_configure().loc`
  — a DIFFERENCE, the only geometric fact that crosses from the X server's coordinate
  space into the compositor's. That is precisely what xwayland-satellite's `create_popup`
  fed an `xdg_positioner`. `X11Popup::new` also mirrors the link onto the popup's own
  surface (`X11PopupLink`), because `find_popup_root_surface` and
  `get_popup_toplevel_coords` recognise a parent chain by the xdg_popup ROLE, which an
  Xwayland surface never has — without it a submenu resolves its root to the menu above
  it and gets a zero offset.

  An X11 popup refuses a popup GRAB (`PopupGrabError::InvalidGrab`): X clients grab
  through the X server and are already holding one. Presenting them as popups is about
  placement and lifetime, not about taking input.

  A window that is NOT presented as a popup still gets parent-relative placement when it
  declares a parent (`window.child::parent_offset`, applied in `on_window_map_initial`),
  for the same reason and by the same difference.

  A FULLSCREEN game is never a popup, and `as_popup` needs two tests to see one: smithay
  reads `_NET_WM_STATE_FULLSCREEN` in its MapRequest arm, which an override-redirect
  window never takes, so an override-redirect window that names NO parent and covers an
  output (the logical output sizes are passed in) is taken as a window on size alone.
  satellite has no such guard.

  Two places the override-redirect distinction still survives, both vendored guards:
  `set_mapped` must never be sent to such a window (enforced in `unmapped_window`), and
  `X11Surface::configure` refuses them, so `shell::flush_pending` routes them through
  `configure_override_redirect`.

  **What still falls through to the window path**, deliberately: a transient DIALOG (not
  ephemeral — a window the user arranged and may return to), and a menu that sets no
  `WM_TRANSIENT_FOR`, which X11 does not require. The latter is the remaining gap;
  satellite's `guess_is_popup` has more signals (Motif hints, skip-taskbar without
  `WM_DELETE_WINDOW`) if it needs closing.

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

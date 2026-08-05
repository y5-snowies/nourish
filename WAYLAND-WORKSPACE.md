# Exposing worlds + zones as Wayland workspaces

Status: **planning only — nothing implemented.** This document is the outcome of a design
discussion; it records what was verified in the tree, what was decided, what is still open,
and the ordered work plan. It is written to be resumable cold.

Every file reference below was checked against the tree at the time of writing. Verify
line numbers before trusting them — the paths are stable, the line numbers may drift.

---

## 1. Goal

1. Status bars (**Waybar**, **sfwbar**) show y5 **worlds** as workspaces, labelled with the
   world name.
2. Once inside a world, its **published zones** appear as workspaces too, alongside the
   worlds, labelled with the zone's member window titles.
3. Optionally, the whole dock surface (workspaces **and** the existing foreign-toplevel
   protocols) is restricted to a single compositor-launched client — a **managed** mode
   modelled on the existing IME exec mechanism.

---

## 2. Protocol choice

**`ext-workspace-v1`** (staging). This is the only protocol that both target bars consume.

- **Waybar** — module `ext/workspaces`, added by
  [PR #4016](https://github.com/Alexays/Waybar/pull/4016) (merged 2025-08-08), replacing the
  old `wlr/workspaces`. Requires `wayland-protocols >= 1.39` at Waybar build time, and
  Waybar must be built with `-Dexperimental=true` for the module to exist. **Most distro
  packages do not set that flag** — check this before debugging an empty bar.
- **sfwbar** — ships `protocols/ext-workspace-v1.xml`
  ([repo](https://github.com/LBCrion/sfwbar/tree/main/protocols)), alongside
  `ext-foreign-toplevel-list-v1` and `cosmic-workspace-unstable-v1`.

Rejected alternative: emulating sway's IPC socket. It is the most battle-tested path for both
bars, but it is a far larger surface (unix socket + JSON tree + event subscription stream) and
is no longer necessary now that Waybar's `ext/workspaces` has landed.

### Bindings are already available — no vendoring, no XML, no version bump

- `vendor/smithay/Cargo.toml:60` pulls `wayland-protocols 0.32.12` with the `staging` feature.
- That crate contains `protocols/staging/ext-workspace/ext-workspace-v1.xml` and generates
  the module `wayland_protocols::ext::workspace::v1::server` (`src/ext.rs:142`).

### Interfaces to implement

| Interface | Notes |
|---|---|
| `ext_workspace_manager_v1` | global; events `workspace_group`, `workspace`, `done`, `finished`; requests `commit`, `stop` |
| `ext_workspace_group_handle_v1` | `capabilities`, `output_enter/leave`, `workspace_enter/leave`, `removed`; request `create_workspace` |
| `ext_workspace_handle_v1` | `id`, `name`, `coordinates`, `state`, `capabilities`, `removed`; requests `activate`, `deactivate`, `assign`, `remove` |

---

## 3. What already exists in the tree

### 3.1 The pattern to copy — hand-rolled foreign-toplevel

`compositor.support/support.smithay/smithay.state/state.foreign/foreign.base/base.rs` is a
hand-rolled `zwlr_foreign_toplevel_manager_v1` **plus** smithay's `ext_foreign_toplevel_list_v1`.
It already demonstrates every piece the workspace protocol needs:

- manager resource list + per-entity handle vec + a mirror `HashMap`
- `reconcile()` that diffs against a source of truth and emits **only deltas**
- an inbound `requests: Vec<(WlSurface, ForeignRequest)>` outbox drained by the rim
- a boot-snapshot enable/disable gate that creates or withholds the registry global
- `teardown()` when muted

Factory: `state.foreign/foreign.factory/factory.rs`.

### 3.2 Where it plugs in

- **Dispatch struct** — `compositor.support/support.smithay/smithay.dispatch/dispatch.state/state.base/state.rs`,
  `pub struct Dispatch` (~line 66) has `pub foreign: ForeignToplevel`. All smithay handler
  impls live in this crate (orphan rule).
- **Rim reconcile** — `compositor.support/support.smithay/smithay.dispatch/dispatch.wire/wire.base/wire.rs`:
  `reconcile_foreign_on_world_change()` (:211), `on_world_switched()` (:220),
  `foreign_reconcile()` (:228), `drain_protocol()` (:282).
- **World-switch hook already wired** —
  `compositor.kernel/kernel.loader/loader.main/main.execute/execute.base/main.rs:300`
  subscribes the rim handler to the `WORLD_SWITCHED` bus channel. Event-driven, not polled.

### 3.3 Worlds

- `compositor.orchestration/orchestration.world/world.manager/manager.base/manager.rs` —
  `WorldManager` with `ids()`, `active_id()`, `spawn_target()`, `set_spawn_target()`,
  `switch()`. World UUIDs are **stable across restarts** (static worlds have fixed UUIDs;
  picker-created worlds get `Uuid::now_v7()` and are persisted).
- **Display names** live in the picker, not on `World`. Every picker/loader world is literally
  `World { name: "world" }`. The user-facing name is
  `picker.state/state.base/base.rs:28` — `world_names: HashMap<Uuid, String>`, edited via
  `picker.surface/surface.handle/handle.rs:23` (`rename_selected`), with
  `picker.name` → `pool::random_name(id)` as a deterministic fallback.

### 3.4 Zones — live, and richer than they first look

Zones **are** implemented and wired (an earlier read of this tree missed it because consumers
import `Zone`/`ZoneSpecifier` directly, never `CameraZone`).

- Type: `y5.camera/camera.zone/zone.state/state.rs` —
  `CameraZone { zone: HashMap<String, Zone> }`,
  `ZoneSpecifier::{ Element { UUID: Vec<Uuid> }, Camera { position, zoom } }`.
- Owner: `y5.camera/camera.state/state.base/state.rs:8` — `Camera.zone`.
- Driver: `y5.canvas/canvas.input/input.keyboard/navigator.rs`, `zone_delegate()` (:62-178).
  Keys at :309-320 — `Super+1..6` recall, `Super+Shift+1..6` set. Six fixed slots keyed
  `"f1".."f6"`; the key **is** the identity, there is no zone name and no publish flag.
- Set semantics (:66-95): if the selection is non-empty → `Element` with the selected window
  UUIDs **captured at set time**; if empty → `Camera` bookmark of the current position/zoom.
- Recall semantics (:104-176): resolve to a `Travel` on the navigator.
- Known TODO at :64-65 — closed windows leave dangling UUIDs; harmless today because recall
  just filters non-matches.

### 3.5 The viewport tree — the part that complicates addressing

`y5.viewport/viewport.state/state.base/state.rs`:

- `OutputViews { map: HashMap<String /* EDID key */, Viewports>, current, .. }` (:81) —
  **every physical monitor has its own independent `Viewports` tree**, its own splits, its own
  cameras, therefore its own six zones.
- `Viewports { root, floating, active: SlotId, pointer: SlotId, next_id, visible }`.
- `Slot { id: SlotId, camera: Camera, content, weight }` (:23) — **each slot owns a `Camera`**,
  so zones are per-slot, not per-world.
- `SlotId = u64` (:7). Allocated monotonically from `next_id`
  (`viewport.interface/interface.base/interface.rs:19,32` — a split consumes two fresh ids, no
  reuse). Persisted in `SlotRec.id` + `ViewportsRecord.next_id`.
- **Critical:** every `Viewports::default()` starts at slot `0` with `next_id: 1` (:169-171).
  So slot 0 exists in *every* world on *every* monitor. `SlotId` is **stable but not unique**.
- The EDID key starts as `""` (bootstrap) and `OutputViews::ensure()` **moves** that tree to
  the real key when the first real output appears (:143-153).

**Zones are not persisted** — `viewport.persist/persist.doc/base.rs:3`:
"Momentum/zone are transient (not persisted)."

### 3.6 Focus accessors (how the rim reaches any of this)

`compositor.orchestration/orchestration.core/core.state/state.base/state.rs`:
`camera()`/`camera_mut()` (:575-597), `active_camera()` (:606), `viewports()`/`viewports_mut()`
(:618-628), `output_views()`/`output_views_mut()` (:633-644), `navigator()`/`navigator_mut()`
(:673-681), `canvas()` (:662), `select()` (:691).

All resolve off `worlds.spawn_target()` + `current_output_key()`. The `workspace.lint.js`
WORLD-ID rule **forbids literal world ids in rim code** — everything must go through these
accessors or `WorldManager::spawn_target()`.

`select()` returns `CanvasSelect { Selection: Vec<Arc<Window>>, Primary: Option<Arc<Window>> }`
(`y5.select/select.state/state.base/select.rs`).

### 3.7 The IME exec mechanism (the model for `managed` mode)

Config: `model.environment/environment.preference/preference.base/base.rs` —
`ime: Option<Ime>` (:189-195), `Ime { exec: String, args: Vec<String> }` (:511-520).

Launch + auth: `support.smithay/smithay.state/state.text.input/text.input.launch/launch.rs`:

- Spawned once from `loader.main/.../main.rs:506-510`, **after** `WAYLAND_DISPLAY` is exported
  (the child inherits it).
- Direct child with `process_group(0)` → pgid == child pid. Records `IME_PGID` **and**
  `IME_START` (`/proc/<pid>/stat` start-time), which is what makes pgid matching safe against
  pid recycling.
- `is_authorized(client, dh)` (:66): leader alive **and** start-time matches, then the client's
  own pid == pgid, or the client's pgid (stat field 5) == pgid.
- Constraint: the child must **not** daemonize, or the connecting process is a reparented
  grandchild and the pgid link is lost.
- Wiring: smithay's `can_view` filter on the manager globals
  (`text.input.factory/factory.rs:53,60`). Unauthorized clients get **no error** — the global
  is simply absent from their registry.

### 3.8 Current protocol gate

`preference.base/base.rs:225-231`:
- `protocol_foreign: String` — `"enabled"` vs anything else. Boot snapshot, **no hot reload**.
- `protocol_foreign_all_worlds: bool` — live-toggleable scope.

Edited in the settings **Misc** tab
(`compositor.extension/compositor.configurator/configurator.settings/settings.surface/surface.misc/misc.rs`).

---

## 4. Decisions taken

### 4.1 Worlds

- Advertise every world from `WorldManager::ids()`, filtering out `LOCK_WORLD` and
  `PICKER_WORLD`.
- `id` = world UUID (stable across restarts — good, bar-side per-workspace config survives).
- `name` = `world_names[uuid]`, falling back to `picker.name::pool::random_name(uuid)`.
- Exactly one world carries the `active` state bit (= `spawn_target()`).
- Capabilities: `activate`. (`assign` only if window-move-to-world is implemented later.)
- One `ext_workspace_group_handle_v1` holding **every** output — y5 worlds span all outputs,
  they are not per-output.
- `activate` → the existing world-switch path
  (`picker_interface::enter` + `set_spawn_target_world`), routed via a `WireTrait` method.

### 4.2 Zones

| Question | Decision |
|---|---|
| Scope | Publish **all zones of the active world** (all viewports, all its monitors). |
| Config tiers | `all_worlds` / `active_world` (default) / `active_viewport`. The last one needs a viewport-change hook — **deferred**. |
| "Published" | Occupied slot = published. No new publish flag, no new UI. |
| Name — `Element` zones | Joined titles of the member windows. |
| Name — `Camera` zones | `"Zone 1".."Zone 6"`. |
| Name — cross-world | Prefix with the world name, **only** under the `all_worlds` config. |
| Name freshness | **Cached.** Computed at zone-set and re-published on zone-activate only. Not recomputed per reconcile. |
| Empty zones | An `Element` zone whose windows have all closed is **unpublished** (`removed`). |
| `active` bit | Never set on zone handles. |
| Persistence | None. Zones stay transient. |
| Capabilities | `activate` only. |

**Zone id — revised.** Four levels, not three, and the middle one is not a UUID:

```
{world_uuid} / {output_edid_key} / {slot_id} / {zone_key}
```

Both the world and output prefixes are mandatory — `SlotId` is stable within a session (and
across restart, since it is persisted), but **not unique**: slot `0` exists in every world on
every monitor, so `{world}:{slot}` collides the moment a second monitor exists, not just under
the all-worlds config. Opaque to the bar, which is what the protocol wants.

Startup wrinkle: the EDID key is `""` until the first real output is identified, at which
point `OutputViews::ensure()` moves the tree. Either withhold advertisement until the key is
non-empty, or accept a one-time remove+add at boot.

### 4.3 Managed vs unmanaged

- `protocol_foreign` is already a `String`, so **widen it in place** to
  `"disabled" | "enabled" | "managed"`. `"disabled"`/`"enabled"` keep their meaning; existing
  `preferences.json` files are unaffected.
- `"managed"` reveals an exec field (`{ exec, args }`, same shape as `Ime`) and applies the
  IME-style pgid+start-time `can_view` filter to **all three** globals: the wlr foreign
  manager, the ext foreign-toplevel list, and the new ext-workspace manager.
- **Zero vendor changes required:**
  - ext foreign-toplevel list — `ForeignToplevelListState::new_with_filter` already exists
    (`vendor/smithay/src/wayland/foreign_toplevel_list/mod.rs:302`); `foreign.factory` currently
    calls plain `new`. One-line swap.
  - wlr foreign manager + ext-workspace manager (hand-rolled) — override
    `GlobalDispatch::can_view`, a provided trait method defaulting to `true`
    (`wayland-server-0.31.14/src/global.rs:119`), reading a predicate out of the global data.
    Those impls already live in `dispatch.state/state.base`.

---

## 5. Open questions — decide before implementing

1. **Respawn policy (highest priority).** IME deliberately does *not* respawn; if it dies,
   `is_authorized` returns false for everyone permanently until the compositor restarts. A
   status bar dying is routine, and "your bar is gone until you reboot the compositor" is a bad
   outcome. Options: keep IME semantics / respawn with backoff / a "restart" button in settings.
2. **Shutdown ownership.** IME is spawned detached and left to the SIGCHLD reaper; nothing kills
   it on compositor exit. Same for the bar, or does `managed` own its lifecycle end to end?
3. **Double-launch trap.** Users normally autostart their bar from the session. Under `managed`,
   a second copy sees *no global at all* — presenting as "waybar shows nothing" with no error
   anywhere. Docs-and-warning-copy problem, or something to detect?
4. **Group layout under `all_worlds`.** Do world handles and zone handles share one flat group,
   and how do `coordinates` order them so worlds don't interleave with zones? Presentation only.
5. **Config granularity.** One enum (`active_viewport` / `active_world` / `all_worlds`) covering
   both worlds and zones, or independent scopes? One enum is simpler; no use case seen for the
   cross product.
6. **Live-apply.** `protocol_foreign` is a deliberate boot snapshot. Adding `exec` inherits
   that — editing it requires a reboot. Acceptable, but the settings page must say so.

---

## 6. Consequences worth remembering

- **Name caching has one visible cost.** A zone whose windows have all closed stays advertised
  with a stale name until someone activates it — the bar shows a button that vanishes when
  clicked. Fix: add **window-close** to the trigger set. It is already event-driven, it is where
  the `navigator.rs:64-65` TODO lives anyway, and it makes unpublish timely for free.
  **Recommended: fold it in.**
- **`Camera` zones can never go empty**, so the unpublish-on-empty rule only ever fires for
  `Element` zones. `"Zone 1".."Zone 6"` are permanent once set, until overwritten.
- **`managed` is authentication, not confinement.** pgid + start-time proves *which process*;
  it does not confine it. Everything in the bar's process group can bind those globals — a bar
  that spawns a shell on click hands that shell the same access. For an IME that leniency is
  intentional (forked helpers). Fine here too, but do not describe it as a sandbox.
- **`ext-workspace` is transactional**, unlike the existing wlr foreign-toplevel code. Client
  requests are queued and applied only on `ext_workspace_manager_v1.commit`, and every event
  batch terminates with a **manager-level** `done`. The existing wlr code emits per-handle
  `done` immediately — do not copy that part.

### Zone activation — the ordering constraint

Everything routes through focus accessors keyed off `spawn_target()` + `current_output_key()`,
so activating a zone outside the currently-focused (world, output, slot) means **retargeting
focus first**. Three steps, in order, each skipped when already correct:

1. **World** ≠ `spawn_target()` → existing switch path. Fires `WORLD_SWITCHED`.
2. **Output key** ≠ current → `output_views_mut().set_current(key)`. Note `current` is normally
   driven by the cursor, so after a bar click the input systems' target output will not match
   where the pointer is until the next pointer motion. Accept this; warping the cursor is the
   over-complicated branch.
3. **Slot** ≠ `viewports.pointer` → set `pointer` (and `active`) to it, then run the existing
   `zone_delegate(…, false)` recall body.

**The constraint:** `navigator_mut()` is the *focused world's* `NAVIGATOR` slot, and
`NavigatorOutput` is applied by the camera system to whatever slot is focused when it reads. So
there is exactly one in-flight `Travel` per world and it lands wherever focus points at read
time. Steps 1-3 must fully settle **before** the `Travel` is set. Since `WORLD_SWITCHED` is a
bus event, a cross-world zone activation cannot set the Travel in the same statement — it needs
a small pending-activation outbox drained after the switch. That is exactly the shape
`request_activation` / `ActivationOrigin::Foreign` already uses for foreign-toplevel, so it is a
copy, not a new pattern.

### Reconcile triggers (fully event-driven, no per-frame cost)

world switch (`WORLD_SWITCHED`, already wired) · zone set · zone activate · window close ·
output add/remove · viewport split/close.

---

## 7. Work plan

Ordered. Each phase is independently landable.

### Phase 0 — decisions
Answer §5. Nothing below is blocked on §5.4-5.6, but §5.1-5.3 gate Phase 4.

### Phase 1 — `ext-workspace-v1`, worlds only
1. New state crates under
   `compositor.support/support.smithay/smithay.state/state.workspace/` mirroring
   `state.foreign` (`workspace.base` + `workspace.factory`). The 30-100 LOC single-module
   policy means this splits across several crates — realistically 6-10 for
   mirror / emit / request-queue / factory.
2. `Dispatch` impls in `dispatch.state/state.base` (orphan rule):
   `GlobalDispatch<ExtWorkspaceManagerV1>`, `Dispatch<ExtWorkspaceGroupHandleV1>`,
   `Dispatch<ExtWorkspaceHandleV1>`; add `pub workspace:` to `struct Dispatch`.
3. `WireTrait` accessor for the roster — the support layer cannot depend upward on the y5
   expansion where `world_names` lives, and the WORLD-ID lint forbids literal world ids. Add
   e.g. `fn world_roster(&self) -> Vec<WorldEntry { id, name, active, coordinate }>`,
   implemented up in orchestration/y5. Same trick as `host_space()` / `all_world_spaces()`.
4. `workspace_reconcile()` in `wire.base` next to `foreign_reconcile()`; subscribe it to
   `WORLD_SWITCHED` alongside the existing handler at `main.rs:300`.
5. Implement the transactional request queue + manager-level `done`.
6. Inbound `activate` → outbox → drained in `drain_protocol()` → world switch via `WireTrait`.
7. New preference `protocol_workspace` (boot snapshot, mirroring `protocol_foreign`) + Misc-tab
   toggle.
8. `./link.all.sh` from the repo root.

**Verify:** Waybar built with `-Dexperimental=true`, module `ext/workspaces`; and sfwbar.

### Phase 2 — prerequisite zone fix
Prune dangling window UUIDs from `Element` zones on window close
(`navigator.rs:64-65`). Standalone, useful on its own, and required before zones are published.

### Phase 3 — zones as workspaces
1. Zone roster accessor: walk the active world's `OUTPUT_VIEWS` → every output's `Viewports` →
   every slot → its six zone slots. Build the four-part id.
2. Name cache: compute on set / activate / window-close; store the string in the mirror.
3. Emit zone handles into the same group, ordered after worlds via `coordinates`.
4. Activation: the three-step focus retarget + pending-activation outbox (§6).
5. Config tier `all_worlds` / `active_world`; world-name prefix under `all_worlds` only.
6. Wire the remaining reconcile triggers.

Deferred: the `active_viewport` tier (needs a viewport-change hook).

### Phase 4 — managed mode
1. **Extract the managed-process mechanism.** IME's auth is a singleton (two file-level statics,
   one process, one predicate). A second managed process needs its own independent pair. Pull
   out a small reusable crate — spawn with `process_group(0)`, pin start-time, `is_authorized` —
   instantiated per slot, and migrate IME onto it.
   > **This is a refactor of working security code and the one place in this plan where a
   > mistake is a keylogger, not a cosmetic bug.** Land and verify it as its own
   > behaviour-preserving step, separately from everything else.
2. Widen `protocol_foreign` to `"disabled" | "enabled" | "managed"`; add the exec/args field.
3. Apply the filter: `ForeignToplevelListState::new_with_filter` in `foreign.factory`;
   `can_view` overrides on the two hand-rolled manager globals.
4. Spawn the managed client from the loader at the same point as the IME (after
   `WAYLAND_DISPLAY` export).
5. Settings-page copy: reboot-to-apply, the no-daemonize constraint, and the double-launch
   warning.

---

## 8. Repo mechanics — do not forget

- **`./link.all.sh` from the repo root** after adding/removing/renaming any crate, or
  cross-workspace `[path]` links go stale.
- Use the **`add-crate` skill**; do not hand-create crate dirs or `Cargo.toml`.
- Crates are **FLAT** — `lib.rs` plus at most one module file, no `src/`. Every dependency is
  `{name}.workspace = true`; paths/versions/features live only at the workspace root.
- **30-100 LOC single-module size policy** — this is why the protocol work fans out into many
  small crates rather than one file.
- **WORLD-ID lint** — rim code resolves the focused world via the Orchestrator focus accessors
  or `WorldManager::spawn_target()`, never a literal world id. The allowlist is empty.
- **Logging** — `error!`/`warn!`/`info!`/`trace!`/`abort!` from
  `compositor_model_debug_instance_record`. Never `tracing` or `log`. Use the `logging` skill.
- **Build** — use the `environment/` scripts (shared target dir), not raw per-workspace
  `cargo build`. Never set `RUSTFLAGS` (it drops the mold linker).

---

## 9. Sources

- [Waybar PR #4016 — port `wlr/workspaces` to `ext-workspace-v1`](https://github.com/Alexays/Waybar/pull/4016)
- [Waybar Workspaces module wiki](https://github.com/Alexays/Waybar/wiki/Module:-Workspaces)
- [sfwbar protocols directory](https://github.com/LBCrion/sfwbar/tree/main/protocols)
- `ext-workspace-v1.xml` — vendored at
  `~/.cargo/registry/src/*/wayland-protocols-0.32.12/protocols/staging/ext-workspace/`

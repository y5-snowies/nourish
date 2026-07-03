# VIEWPORT.md — multi-output (per-monitor viewport) model & known gaps

This document describes y5's multi-monitor model and, more importantly, the
**known gaps / deferred work** discovered while landing it. It is the running
punch-list for finishing multi-output — read it before touching the render loop,
the pointer/hit path, DRM scanout, or capture.

## Model (what multi-output means here)

- **Each physical output is its own viewport.** Every monitor has its OWN camera
  (independent pan/zoom) onto the ONE shared y5-world. NOT extended desktop, NOT
  mirrored. Per-output view state lives in `OutputViews { map, current }`
  (`y5.viewport/viewport.state/state.base`), keyed by the monitor's **EDID
  identity** (`output_key` = `"make model serial"`, connector-name fallback when
  EDID is unreadable — see `orchestration.core/core.state/state.base::output_key`).
- **The kernel drives N CRTCs.** `NativeRenderContext.outputs: Vec<OutputPipe>`,
  one pipe per lit CRTC (`kernel.native/native.context/context.render/render.base`).
  The render loop iterates pipes; vblank routing is by `crtc::Handle`.
- **The settings layout canvas is a cursor-teleport map only** — square positions
  are teleport geometry, decoupled from each monitor's camera. Cursor crossing an
  edge teleports per `TeleportLayout` (`orchestration.seat/seat.pointer/pointer.teleport`).
- **`render_output` vs `cursor_output`/`active_output`.** During a draw the render
  loop sets `Orchestrator.render_output` to the pipe being drawn; the focus /
  coordinate accessors (`current_output()`) resolve it. During input it is `None`,
  so those accessors resolve `cursor_output`. Screen-space surface creation/gating
  uses `active_output()` = `cursor_output ?? primary`.

## Pacing (per-monitor refresh)

Each output renders **only on its own vblank**, so a 144 Hz monitor is not paced
by a 60 Hz neighbour:

- `execute(scope: RenderScope)` — `Crtc(handle)` from the vblank path renders ONLY
  the flipped pipe; `All` (ping / kickstart / resume) renders every idle pipe.
- `OutputPipe.in_flight` — set on a successful queue, skipped in the loop, cleared
  on that pipe's vblank and on session resume. smithay `queue_frame` double-buffers,
  so the skip just avoids wasted CPU renders.
- The Law-7 timing features (`flip-estimate`/`timing-predict`/`timing-throttle`)
  are **OFF** in the shipping build; the global `refresh` in
  `native.wire/wire.frame/frame.base/frame.rs` is dead at runtime (see gap R5).

---

# Known gaps / deferred work

Ordered roughly by priority. Each item: symptom → cause → where → suggested fix.
Items marked **[workaround live]** have a correct-but-suboptimal fix shipping now;
the entry describes the efficient replacement.

## R — Rendering / damage / sync

### R1. Partial-damage rendering is disabled on multi-output **[workaround live]**
- **Symptom:** world content blinks/clears with ≥2 monitors — heavy on Vulkan,
  milder on GLES.
- **Cause:** smithay's `OutputDamageTracker` does age-based partial rendering
  (clears only damaged rects, SKIPS elements not intersecting damage, trusting the
  aged swapchain buffer to still hold the remainder). Once each output is paced on
  its own vblank the swapchains sit at age ≥ 1, so partial damage is the norm — and
  the renderers don't honor the contract (see R2).
- **Workaround shipping:** when `outputs.len() > 1`, force full damage each frame via
  `drm_output.with_compositor(|c| c.reset_buffer_ages())` in
  `kernel.native/native.render/render.execute/execute.base/execute.rs` (search
  `reset_buffer_ages`). Full clear + all elements = always correct; single output
  keeps the optimisation.
- **Efficient fix (deferred, needs hardware):** make the renderers honor damage
  (R2), then drop the `reset_buffer_ages` call.

### R2. Vulkan frame ignores damage rects (full-attachment clear)
- **Cause:** `VulkanFrame::clear` ignores its `_at` rects and stores a full-target
  clear; `render_texture_from_to` ignores `_damage`; `record_composition` always
  clears the whole attachment. So a partial frame blanks everything not in `ops`.
- **Where:** `kernel.vulkan/vulkan.renderer/renderer.core/core.base/frame.rs:131`
  (`clear`), `frame.rs:154` (`render_texture_from_to`),
  `.../renderer/submit.rs` (`record_composition` call), and the record helper
  `kernel.vulkan/vulkan.command/command.record/record.base/record.rs`.
- **Fix:** honor `at`/`_damage` with scissored clears + `loadOp = LOAD` so untouched
  regions of the aged buffer are preserved (mirroring GLES). Requires the target
  image transition to PRESERVE content (not `old_layout = UNDEFINED`) — see the
  import path in R4. Contract reference: vendored
  `vendor/smithay/src/backend/drm/compositor/mod.rs` (age accumulation ~2176-2246)
  and `vendor/smithay/src/backend/renderer/damage/mod.rs` (clear ~881, element skip
  ~906-912, old-damage ~741-760).

### R3. Shared iced dmabuf has no producer→consumer fence
- **Symptom:** iced surface flicker/tearing; contributes to R1 (worse on Vulkan
  because compositor device ≠ wgpu device).
- **Cause:** each iced surface owns ONE dmabuf written by wgpu and sampled by the
  compositor; iced's `render_into` wait/poll is commented out (fire-and-forget), so
  both outputs sample a buffer another GPU queue may still be writing, with no
  cross-device sync.
- **Where:** `monitor.runtime/runtime.surface/surface.base/src/surface.rs` (single
  dmabuf), `monitor.compositor/compositor.iced/iced.base/src/instance.rs`
  (`element_in` clones the same dmabuf each frame), `support.iced/iced.core/core.engine/engine.base/src/runtime.rs`
  (commented-out wait/poll ~388-409).
- **Fix:** add a wgpu→Vulkan acquire fence/semaphore for the shared dmabuf (bridge
  through `vulkan.sync`), or double-buffer the iced dmabuf per producer frame.

### R4. `import_dmabuf` has no cache
- **Symptom:** wasteful — each output pass re-imports the iced dmabuf into a fresh
  `VkImage` with a full layout transition + `device_wait_idle`.
- **Where:** `kernel.vulkan/vulkan.renderer/renderer.core/core.base/renderer/import.rs`
  (~19-102).
- **Fix:** cache imported images keyed by dmabuf identity; invalidate on
  size/format/handle change. The FOREIGN-acquire currently uses
  `old_layout = UNDEFINED` (content-discarding) — revisit alongside R2 (LOAD needs
  content preserved).

### R5. Shared camera caches are single-output (bevy + iced) — CONFIRMED, pending decision
- **Symptom:** with ≥2 outputs the world-space items are marked "camera changed" and
  damaged/re-rendered EVERY frame even when nothing moves — the single
  `last_transform`/`last_output_size` flip-flops as the per-output loop passes each
  output's differing camera/size. The iced one re-renders world UI via wgpu every
  frame (feeds the Vulkan blink / R3); the bevy one bumps commits (masked by R1).
- **Where:** `support.bevy/bevy.core/core.frame/frame.base/lib.rs`
  (`cache_camera_and_bump`) and `monitor.compositor/compositor.iced/iced.base/src/registry.rs:635`
  (`cache_camera_and_bump`).
- **Fix (proposed, awaiting confirmation):** key the cache per-output (a
  `HashMap<render-output, (Transform, Size)>`) so an item's commit bumps only when
  THAT output's camera/size actually changes. Note: the commit counter is one-per-item
  shared across outputs' damage trackers, so moving one output's camera still redraws
  the item on the others — full per-output damage isolation is a larger change; the
  per-output cache keying fixes the common static-desktop invalidation.

### R6. Frame-pacing state (`needs_redraw`, predict/throttle) is global
- **Symptom (needs_redraw):** a one-shot change visible only on output B, arriving
  while B is mid-flight, can be missed if output A's vblank consumes the global
  `needs_redraw` with an empty A render. Self-heals during animation (the render
  loop reschedules) and because both cameras view the same world; rare in practice.
- **Where:** `support.smithay/smithay.dispatch/dispatch.state/state.base/state.rs`
  (`needs_redraw`, `render_in_flight`).
- **Symptom (timing nets):** `refresh`, `PresentationClock`, `VblankThrottle`,
  `estimate_slot` in `native.wire/wire.frame/frame.base/frame.rs` are single
  instances seeded from the primary; blend a 120 Hz and 60 Hz stream. Currently
  inert (features OFF) but wrong if enabled.
- **Fix:** per-output `needs_redraw`/dirty (e.g. a generation counter + per-pipe
  `rendered_generation`), and per-CRTC pacing state (map keyed by `crtc::Handle`).

## P — Power / session lifecycle

### P1. DPMS-off blanks ALL outputs
- **Symptom:** on a laptop, closing the lid (or idle-blank) blanks an attached
  external monitor too.
- **Cause:** a single global `DISPLAY_OFF` short-circuits the whole frame.
- **Where:** `kernel.native/native.render/render.execute/execute.base/execute.rs`
  (early `DISPLAY_OFF` return), `native.wire/wire.input/input.base/input.rs`,
  `native.wire/wire.plugin`, `entry.rs` (`DISPLAY_SNAPSHOT`).
- **Fix:** make DPMS-off per-output (per connector — the internal panel is eDP/LVDS),
  gating only that pipe in the loop, not the whole `execute()`.

### P2. `VBLANK_SEEN` is a single global flag across all CRTCs
- **Symptom:** the FIRST output's vblank clears the `resuming` gate for ALL outputs;
  a slower second output whose CRTC hasn't flipped can be treated as resumed and its
  queue torn down; HDR signalling (gated on this flag, applied per-connector) may
  fire before a connector's own first vblank.
- **Where:** written `native.wire/wire.frame/frame.base/frame.rs` (`process_vblank`)
  and `wire.session`; read in `execute.rs` (HDR signal gate, `present` resume gate).
  Token in `orchestration.driver.resume.base`.
- **Fix:** track "seen vblank" per CRTC/output, not globally.

## I — Input / layout

### I1. Layer-shell fallback picks the primary
- **Symptom:** a layer surface that does NOT bind a specific `wl_output` (bars asking
  for "current output") lands on the primary, not the focused monitor.
- **Where:** `support.smithay/smithay.state/state.layershell/layershell.wire/lib.rs`
  (`.or_else(|| outputs().next())`).
- **Fix:** fall back to the active/cursor output. Note: `support.smithay` is below the
  rim and can't call `active_output()` directly — the cursor output would need to be
  plumbed down (or resolved from `OUTPUT_VIEWS.current`, as `HitCx` now does).

### I2. Two hit-test implementations — confirm/align
- `y5.surface/surface.interface/interface.core/hit.rs` (the one whose output
  resolution was fixed via `HitCx::current_output()`) and
  `y5.surface/surface.interface/interface.base/hit.rs` (the newer geometry-iterating
  version). Both crates are widely depended on. Confirm which is live for each caller
  and delete/align the stale one to avoid divergence.

### I3. Dead single-output helpers
- `support.smithay/smithay.state/state.space/space.base/state.rs` — the
  `default_logical` / `default_physical_*` / `default_scale` / `default_output` /
  `default_output_geometry` family, all built on `outputs().next().unwrap()`. No live
  callers found; delete or rework to take an `&Output` before anyone re-adopts them.
- `orchestration.seat/seat.pointer/pointer.input/native_motion/dispatch.rs` —
  `compositor_output_size` from `outputs().next()` appears unused; remove.

### I5. Default/primary-monitor selection removed from the Display tab
- The Display tab's "select a different monitor + Apply" used to route through an
  active-output **switch** gate (`OutputSwitchRequest`) that tore the sole pipe down
  and re-lit a chosen connector — a single-output construct that doesn't fit
  independently-driven multi-output (and identified the pipe by positional `outputs[0]`,
  not a stable id). That whole path — the `OutputSwitchRequest`/`OUTPUT_SWITCH_*` tokens,
  `switch::{apply,finish,arm,revert_to,drain}`, `OutputSwitchBaseline`/`output_revert`,
  and the `Applied.switch` flag — has been **removed**. Resolution changes are now
  always in-place per-pipe mode changes via `display.mode`. `display.reconcile` retains
  only the live hotplug `reconcile`/`add_output`/`bring_up`.
- Consequence: there is currently **no UI to set the default/primary monitor**. The
  persistence helper (`pref::set_default`, still used by the standalone settings-editor)
  and `profiles.first()` startup default remain, so if a "primary monitor" affordance is
  wanted it can persist through those — but it needs a rethink of what "primary" means
  for independently-driven outputs (layout origin? startup default only?).

### I4. Global-space output layout is width-tiled bookkeeping
- Outputs are mapped into the `Space` by deterministic horizontal width-tiling
  (`kernel.graphic/graphic.preference/preference.layout/layout.output` +
  `display.reconcile::add_output`). Because rendering is per-output-camera and
  framebuffer-local, these positions are only for hit-testing / layer geometry / the
  distinct-origin assumption. If a future feature needs real global-space layout
  (vs the teleport map), this is where it lives. Screen-UI code must NOT re-map an
  output at `(0,0)` (that historic bug was fixed in picker/overview/overlay
  world-switch — they now remap ALL outputs at their real positions).

## C — Capture / lock

### C1. Lock captures a single output
- Screen lock captures ONE framebuffer (now the ACTIVE output, was `OutputId(0)`).
  For true multi-output lock each monitor should show the lock/frozen background.
- **Where:** `y5.lock/lock.interface/interface.base/interface.rs`.
- **Fix:** per-output lock capture + per-output lock surface.
- **Note (done):** capture `OutputId` is now a STABLE EDID-derived id
  (`OutputId::from_key`, `y5.graphic/graphic.capture/capture.registry/source.rs`),
  not a positional index; request sites (screenshot/screencast/picker/lock/overview)
  target `active_output()`.

---

## Appendix — landed multi-output work (for context)

Done in the viewport-experimental branch: per-output `OutputViews` cameras;
per-CRTC render scope + `in_flight` pacing; EDID `output_key` identity with
connector fallback; session resume resets/remaps ALL pipes; `send_layer_frames`
per-output; world-switch maps ALL outputs; stable capture `OutputId`; screen-space
input via `HitCx::current_output()`; `apply_pointer` warp via `current_output()`;
menu-bar resize gated to the active output; cursor teleport layout + settings
canvas; multi-output forced full damage (R1 workaround).

Also: **per-window fractional scale** — now computed cross-output from live view
state (each window follows the HIGHEST-zoom viewport across ALL monitors showing it)
and emitted only when a surface's scale changes (`update_fractional` +
`emit_best_per_surface`, dedup via a `FRAC_SENT` thread-local). Fixes the per-output
flip-flop that re-sent `wp_fractional_scale` to clients every frame. (The bevy/iced
camera caches in R5 are the remaining same-class invalidations.)

Also: **secondary-monitor mode changes** — `OutputModeRequest::Apply` carries the
target `edid_key`; `display.mode` resolves + applies/confirms/reverts per-pipe (was
primary-only, so a secondary reverted instantly); `enumerate` reports each driven
pipe's own `current` mode. **Overview overlay is active-monitor-only** — `draw.frame`
`prepare`/`band` gate on `on_active_output()`, which also fixed the embedded picker
Bevy globe rendering (its single `GLOBE_SIZE` thrashed as the per-output loop rebuilt
it between differently-sized monitors every frame).

Also: **per-output active/inactive + teleport reduction/projection** —
`OutputProfile.active` (prefs, default true) marks a monitor DRIVEN or deactivated. The
settings Display resolution list has an "Inactive" first item (deactivate — refused for
the LAST active monitor); clicking a mode on an inactive monitor reactivates it there.
The kernel `reconcile` drives only connected+active monitors (tears down deactivated
ones; all-inactive → fallback to first-in-prefs so it's never dark), triggered on
activate/deactivate via `OUTPUT_RECONCILE_REQUEST` → deferred `switch::drain_reconcile`.
`build_teleport(prefs, connected_keys)` filters the live cursor-teleport map to
active+connected placements — inactive/disconnected ones stay in `outputs_layout`,
hidden on the canvas (`LayoutCanvas::shown`), restored on reactivate/replug (never
pruned; the UI commits the full layout). `TeleportLayout::neighbor` is now ORTHOGONAL
PROJECTION (nearest placement in the ray direction whose span covers the exit point;
gapped/non-snapped monitors cross) with an optional `teleport_cyclic` wrap-around
(Display-tab checkbox). Verified by the `pointer.teleport` tests (12 passing).

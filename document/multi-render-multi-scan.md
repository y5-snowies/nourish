# Multi-render / multi-scan: the `gpu_router` redesign

Status doc for the GPU routing redesign — the `gpu_router` config model, per-node `mode`,
strict-everywhere selection, token-gated option-B (composite-on-render-node), the multi-device
runtime, and the per-surface-feedback policy. Companion to `document/GPU_TOPOLOGY.md` and
`document/GPU_UNTILE_BLIT.md`. The living design + per-stage status is in the approved plan
(`~/.claude/plans/greedy-enchanting-graham.md`).

**One-line state:** compiles tree-wide, JS lint 0-failures, binary clean, and **byte-identical for
a plain single-GPU `render_node` config** by construction. **Nothing multi-GPU or option-B is
runtime-verified** — it is correct-by-construction and needs the split box.

---

## 1. What changed

### Config model (settings.json)
Two mutually-exclusive variants (exactly one; both/neither panics at load in
`config.base::Environment::validate`):
- **Simple** (unchanged on disk): `render_node: String` (+ optional `scanout_node`).
- **Advanced**: `gpu_router: { "<render-node path>": { "mode": [<tokens>], "scanout": "<card>" } }`.

`render_node` became `Option<String>`; existing files still parse. **`experimental.json`'s 11
`gpu_*` flags were migrated into per-node `mode` and REMOVED** — `experimental.json` is no longer
read and the `experimental.base` crate was deleted.

**New crates:** `config.mode` (`ModeToken`/`ModeFlags`/`fold`), `config.router` (`ResolvedRouter`
+ desugar + accessors), `topology.plan` (strict frame-path policy, unit-tested), `topology.feedback`
(per-surface modifier policy, unit-tested), `assemble.secondary` (multi-device open).

### `mode` tokens (per render node)
Polarity mirrors stable — an **empty** `mode` == stable HEAD (no behaviour injected). Tokens:

| token | meaning |
|---|---|
| `session_primary` | the singleton anchor (env var, capture, orchestration GPU, default dmabuf feedback). **Implicit for a single-entry map.** |
| `no_negotiate_modifiers` / `force_linear` / `force_tiled` / `allow_dcc` / `force_multiplane` / `probe_modifiers` / `no_pin_wgpu` / `no_direct_scanout` / `scanout_bridge` | the migrated `gpu_*` flags (same names/polarity). |
| `untile_blit` | render→scanout untiling blit. **Explicit-only, not a floor** — an empty cross-vendor intersection without it panics. |
| `zero_copy` | permit the zero-copy import fast path (else always blit). |
| `local_render` | **option B**: composite on the render node itself and cross to the scanout card. Default (absent) = composite on the scanout card (today's behaviour). |
| `render_discovery` / `scanout_discovery` | opt back into the pre-strict silent fallbacks (else the unresolved/non-KMS case panics). |
| `blit_fallback` | on a failed `zero_copy` import, fall back to blit instead of panicking (needs `untile_blit`). |
| `dmabuf_main_device` | per-surface feedback main-device eligibility (policy only; wiring not landed). |

### Behaviour by stage
- **Strict selection** (`select.scanout::resolve`): unresolved `render_node` / non-KMS `scanout` →
  panic with an actionable message unless `render_discovery`/`scanout_discovery`.
- **Strict blit** (`surface.rs` / bevy): empty cross-vendor intersection → panic unless
  `untile_blit`; a failed blit alloc → panic (the old silent degrade → `BAD_MATCH` is gone).
- **Option B** (token-gated by `local_render`): `assemble.display` opens a GBM on the render node;
  `assemble.renderer` registers both nodes; the compositing renderer is smithay's cross-device
  `gpus.renderer(render, scanout, Argb8888)` (via `StateDRMBinding.cross_device`, `false`=default=
  `single_renderer`=byte-identical) at every DrmOutput site. smithay does the render→scanout copy
  (direct import, else the `copy_format` intermediate = the cross-vendor untile).
- **Multi-device runtime**: `NativeRenderContext.devices: Vec<DeviceRender>` (per-card
  manager/fd/vblank-token); `OutputPipe.device: DrmNode`; vblank routes on `(device, crtc)`;
  session pause/resume iterate all cards; `assemble.secondary` opens every OTHER KMS card at boot
  and lights its monitors locally (additive — no-op on a single-card box).

---

## 2. Honest review

### Regressions found + FIXED
1. **Single-GPU desktop double-open** — `card_node` used the *render* node for `route=None`, but
   `assemble.secondary` skips the primary by *card* `dev_id`; a normal desktop would open its
   primary card twice (phantom device / duplicate vblank / master conflict). Fixed: `card_node` is
   derived from the scanout card *path* (`DrmNode::from_path(device_path)`).
2. **Phantom monitor-less secondary** — `assemble.secondary` opened every KMS card, including the
   render-only dGPU on a PRIME laptop. Fixed: it scans connectors first and skips cards with no
   connected monitor.

### Biggest remaining gap — multi-device *rendering* is unfinished
The frame executor (`render.execute::execute`) builds **one** renderer (the primary's) and uses it
for every output. Secondary outputs are modeset correctly at boot but would render with the wrong
device's renderer at frame time. **True multi-monitor-across-GPUs lights the second monitor but
does not update it live.** Needs the executor to select `single_renderer(output.device)` (or the
cross-device renderer) per output inside the per-output loop. **Does not affect a single-monitor
PRIME box** (the render-only dGPU is skipped, so there is only the primary device/output).

### Regressions on the DEFAULT path (no gpu_router / no experimental)
- **Strict selection panics where stable silently fell back**: an unresolved `render_node` (stale
  path / absent device) now panics instead of a heuristic fallback. Applies to the *simple*
  `render_node` path too, not just `gpu_router`. A correct `render_node` is unaffected.
- **`experimental.json` `gpu_*` flags are dead** (migrate-&-remove) — anyone who used them loses
  that tuning silently.
- Minor: developer Statistics env key `EXPERIMENTAL_GPU_FLAGS` → `GPU_MODE` (display-only).

### Missing / not wired
- Secondary monitors are **not in the settings-UI output snapshots** (`write_snapshots` reads the
  primary manager only).
- **Per-surface dmabuf feedback (E) not wired** — `topology.feedback::advertise` (the ∩+LINEAR
  policy) is implemented + tested, but `set_feedback` on commit is not (the global default feedback
  is a correct fallback). Deliberately not wired blind: it drives client buffer allocation, so a
  mistake could affect the working single-primary path.
- **Runtime GPU hotplug** not wired (boot-time multi-GPU works; needs a `frame::register(EventLoop →
  LoopHandle)` refactor). Deemed low priority.
- `assemble.secondary` skips the Law-7 modifier filter the primary applies (minor).

### Not runtime-verified (fundamental)
- Option-B cross-device present (smithay `MultiRenderer` cross-vendor copy) — never run.
- Multi-device secondary open / per-card DRM master / per-card modeset — never run.
- Everything compiles and is correct-by-construction; there is no runtime/hardware test.

---

## 3. Recommended verification config (GTX 1050 → UHD 620)

First confirm device paths (`ls -l /dev/dri/by-path`, `drm_info | grep -i driver`): this doc
assumes NVIDIA render `= /dev/dri/renderD129`, Intel scanout card `= /dev/dri/card1` — adjust to
match. **Delete/ignore the old `experimental.json`** (no longer read). `session_primary` is implicit
for a single entry.

**Config A — start here (reproduces the confirmed-working untile-blit; lowest risk).** Desktop
composites on Intel exactly like stable; only the iced/bevy bridge surfaces cross NVIDIA→Intel:
```json
{ "gpu_router": { "/dev/dri/renderD129": { "mode": ["untile_blit"], "scanout": "/dev/dri/card1" } } }
```
`untile_blit` is REQUIRED — without it a bridge surface on the empty cross-vendor intersection
panics with an actionable message (was `BAD_MATCH`).

**Config B — the new architecture (option B: composite the whole desktop on NVIDIA, cross to
Intel). Higher risk, unverified:**
```json
{ "gpu_router": { "/dev/dri/renderD129": { "mode": ["local_render", "untile_blit"], "scanout": "/dev/dri/card1" } } }
```
If Intel shows garbage/black, that is the unverified smithay cross-vendor copy (expected to need
tuning). Add `zero_copy` only to test the direct-import fast path (it falls to the blit when
modifiers don't intersect, which cross-vendor they won't).

**Order:** boot A first (should match known-good), then try B.

---

## 3b. Hardware-validation fixes (this session, from real-hardware logs)

Caught by running Config A/B on a GTX 1050 → UHD 620 PRIME laptop:
1. **single-GPU double-open** — `card_node` (`wire.entry`) used the RENDER node for `route=None`;
   `assemble.secondary` skips the primary by CARD `dev_id` → desktop re-opened its own card. Fixed:
   `card_node = DrmNode::from_path(display.device_path)` (a card node).
2. **secondary opened render nodes** — udev DRM enum includes `renderD*`; `assemble.secondary`
   seat-opened them → ENODEV abort. Fixed: filter to `NodeType::Primary` + non-aborting `try_open`
   (new in `seat.interface.open`). Also skips cards with no connected monitor (PRIME dGPU).
3. **option-B render-node open** — `assemble.display` seat-opened the render node (`renderD129`) for
   its GBM → ENODEV (render nodes aren't seat-managed). Fixed: plain `OpenOptions` open (no master).
4. **option-B + Vulkan guard** — `local_render` + `renderer: "vulkan"` black-screened (see §4). Now a
   fail-fast `abort!` at `wire.entry` with an actionable message instead of a silent black screen.

**Validated on hardware:** Config A (`["untile_blit"]`, composite on Intel, untile the bridge
surfaces) boots to the desktop. This is the primary real-hardware validation of the config model,
per-node `mode`, strict selection, the multi-device foundation, and the NVIDIA→Intel untile-blit.

## 4. Vulkan option-B (composite on the render node with `renderer: "vulkan"`) — IMPLEMENTED

**The old problem:** `renderer: "vulkan"` composites with `ctx.vulkan`, a `VulkanRenderer` that
reaches KMS by binding the SCANOUT card's GBM buffer directly (`impl Bind<Dmabuf>`) — so on the
stock path it MUST live on the scanout card. Option B (`local_render`) moved the GLES `gpus` path to
the render node, but left the Vulkan renderer on the scanout card while disabling the bridge
untile-blit it relied on → black screen. The earlier session shipped a fail-fast `abort!` guard for
that misconfiguration.

**GLES-vs-Vulkan inventory:** the ONLY gpu-routing feature that WAS GLES-only is option B. Per-node
`mode` tokens, the untile-blit bridge, strict selection, direct-scanout, deep-color, VRR, capture are
renderer-agnostic. HDR/PQ is Vulkan-only (the opposite gap). Option B is now implemented for Vulkan
too, closing that gap — both approaches, gated on `local_render` + `cross_device`; the abort guard is
removed.

**Now landed (compiles: `y5_compositor` links clean under `backend-native,renderer-vulkan`):**
- `VulkanRenderer::new_for_node(node)` (`vulkan.renderer/.../renderer/lifecycle.rs`) — node-directed
  constructor (via `physical::for_node`).
- `wire.entry`: when `composite_on_render_node()`, `ctx.vulkan` is built with
  `new_for_node(render_node)` (composites on the render GPU); a SECOND `VulkanRenderer` on the scanout
  card is built into `ctx.vulkan_scanout` (`new_for_node(card_node)`), `None` if that device fails
  (→ approach A). The abort guard is gone.
- `OutputPipe.vk_offscreen: Option<(Dmabuf, Mode)>` — the per-output render-node offscreen (realloc on
  resize), mirroring winit's `context.vulkan_target`.
- `render.execute::present_option_b` — the frame-path redirect. When `cross_device`, every DISPLAYED
  Vulkan `render_frame` (single-pass + lock-fade Pass 2) is routed through it; the single-device path
  is untouched (byte-identical). Capture Pass 1 composes into the offscreen too (capture copy is
  `vk.finish()`'s side effect) but does not hand off.

`present_option_b` implements BOTH approaches in one path — **no `Y5_VK_DIAG` debug staging**:
1. `vk` (render node) composes the scene into `vk_offscreen` — `Bind::bind` → `vk.render` → draw
   elements back-to-front → `finish()`→SyncPoint → `sp.wait()`.
2. **Approach B** (preferred): if `ctx.vulkan_scanout` exists, import the offscreen into it
   (`import_dmabuf`) and `drm_output.render_frame(scanout_vk, [TextureRenderElement], …)` — a
   GPU-side blit on the scanout card that does the untile, no CPU copy.
3. **Approach A** (fallback): if the scanout Vulkan device is absent or its import fails (cross-vendor
   acquire unsupported), import the offscreen into the cross-device GLES `MultiRenderer`
   (`gpus.renderer(render, scanout, Argb8888)`) and `render_frame` that — smithay does the
   NVIDIA→Intel copy (direct import when modifiers intersect, else `copy_format`).

`render_frame` is generic over the element type, so the handoff wraps the imported offscreen as a
plain `TextureRenderElement` — no new element enum. Both approaches share the offscreen-compose step
and differ only in which renderer imports + scans out.

**Still hardware-in-the-loop (unverified without the split box):** whether the NVIDIA→Intel Vulkan
import actually samples correctly (approach B) or falls through to GLES-cross (approach A), and the
texture transform (approach A's GLES sampling may need `Flipped180` like the winit blit — currently
`Normal`; adjust on hardware if the image is inverted). The code paths compile and are wired; the
pixel correctness is a hardware validation, not missing code.

## 5. Vulkan runs at 30 fps where GLES holds 60 (present-sync bottleneck)

Observed on real hardware: with `renderer: "vulkan"` the compositor caps at ~30 fps where GLES is a
stable 60. Reproduces in **every** Vulkan configuration:
- upstream-integration stable, single GPU, rendering on Intel;
- this worktree, single GPU, rendering on `renderD128`;
- this worktree, the NVIDIA→Intel split with `local_render` + `untile_blit`.

Exactly-half framerate is the signature of a per-frame CPU stall that pushes render past the vblank
deadline: the pipeline can't overlap render with scanout, so it slips one vblank and locks to
every-other-vblank.

### Root cause — *when* the page-flip is committed relative to the vblank

The GPU render itself is fast enough for 60 in both renderers; the difference is **when the KMS
page-flip is committed**, which decides whether it catches the very next vblank or the one after.

The frame loop is vblank-driven (`wire.frame/frame.rs::process_vblank`): a pipe's flip completing
delivers a vblank → `in_flight = false` (frame.rs:250) → if a redraw is pending, `execute()` renders
THAT output immediately, inside the vblank handler → `present()` queues the flip → `in_flight = true`
(`render.execute/execute.base/execute.rs::present`). A pipe that is `in_flight` is SKIPPED by the
render loop until its own next vblank. So the cadence is set entirely by *which* vblank each flip lands
on.

`VulkanRenderer::submit_frame` (`vulkan.renderer/renderer.core/core.base/renderer/submit.rs`) has two
present paths, selected by `use_native_fence()`:

- **Synchronous (the DEFAULT, `renderer_sync` unset):** `render_frame` does not return until
  `submit_frame` has called `device_wait_idle()` (submit.rs:347) — it blocks the CPU until the GPU is
  100 % done. Only *then* does `present()` commit the flip, and it returns `SyncPoint::signaled()` so
  there is no `IN_FENCE`. Because the render was kicked off BY the vblank and the CPU sat blocked
  through the whole GPU render, the flip is committed late in the frame period — past the driver's
  latch window for the imminent vblank — so it scans out at the vblank AFTER next. The `in_flight`
  gate then holds the next render until that late flip completes, and the loop settles into a stable
  **one render per two vblanks = 30 fps**. Stat/log: `synchronous (device_wait_idle)`.
- **Native KMS `IN_FENCE` (opt-in, `renderer_sync: "infence"`):** submit signalling a binary render
  semaphore (and a pacing `VkFence`) **without** `device_wait_idle`, export the semaphore as a
  `sync_file` fd (`vulkan.sync/sync.export`), and return it as the `SyncPoint`. `render_frame` now
  returns while the GPU is still rendering, so `present()` commits the flip at the START of the frame
  and hands KMS that fd as the atomic commit's `IN_FENCE`. The hardware — not the CPU — holds the flip
  until the fence signals; the GPU finishes the render within the frame; the flip lands on the very
  next vblank. New frame every vblank = **60 fps**. Stat/log: `native KMS IN_FENCE (sync_file)`.

**Why GLES is 60 and Vulkan is 30, precisely:** smithay's GLES present is always the async-fence path
— `render_frame` returns a *pending* EGL fence, the flip is committed early with that fence as the
`IN_FENCE`, and the GPU render overlaps the previous frame's scanout. The flip therefore always makes
the next vblank. Vulkan's default does the opposite: it CPU-blocks on the full GPU render before
committing, so the flip commit lands too late for the next vblank, slips one, and the vblank-gated
`in_flight` loop locks that slip into a permanent 2:1 cadence. It is **not** that the Vulkan GPU work
is too slow — it is that blocking on completion before arming the flip pushes the commit past the
latch deadline. Async fencing lets the flip be armed early and the hardware absorb the render time;
that is the entire 60-vs-30 difference, and it is intrinsic to the Vulkan submit — independent of GPU
routing, card count, and option B. All three failing configs above end in the same `submit_frame`.

**Contributing factor (vulkan_mode only):** every Vulkan-mode frame ALSO runs a GLES `prepare()`
(bevy/iced/parallax) before the Vulkan composite (`execute.rs` builds the scene on both renderers), so
there is more per-frame GPU work than the GLES-only path, and the synchronous drain serializes against
it. This widens the window in which the flip commit lands late. A frame-time trace (proposition 5
below) is the way to confirm how much each factor contributes.

### The concrete GLES-vs-Vulkan difference: smithay-native fencing vs hand-rolled fencing

The real distinction is **not** that Vulkan does extra work — it is that GLES gets the async KMS fence
for free from smithay and the y5 Vulkan renderer does not:

- **GLES = smithay's own `GlesRenderer`.** Its `render_frame` returns a `RenderFrameResult` whose
  `SyncPoint` is a real EGL native fence (`export_sync_point()` / `EGL_ANDROID_native_fence_sync`);
  `DrmCompositor` attaches it as the atomic commit's `IN_FENCE` automatically. So `needs_sync()` is
  false, `honor_needs_sync` is a no-op, the flip is committed early, the hardware waits on the fence →
  60 fps. **This is built into smithay and does not consult `renderer_sync` at all.** (The unused
  in-tree `gles.sync/sync.export` crate is a standalone re-export of that same capability; the native
  GLES scanout path gets it from smithay directly — see the note in that file.)
- **Vulkan = y5's custom `VulkanRenderer`.** smithay knows nothing about it and cannot pull a fence out
  of it, so y5 must hand-roll the fence→KMS handoff. Two hand-rolled options:
  - default (`renderer_sync` unset): NO fence — `device_wait_idle()` on the CPU + return
    `SyncPoint::signaled()`. Late commit → 30 fps.
  - `infence`: manually export the Vulkan render semaphore as a `sync_file` fd and return it as the
    `SyncPoint`, so smithay attaches it as `IN_FENCE` — i.e. hand-replicate exactly what GLES gets for
    free. Early commit → 60 fps (when the export works).

So `renderer_sync`/`infence` exists ONLY to give the Vulkan renderer the fence path GLES already has
natively; GLES never had the gap. **In the current tree `renderer_sync` is read solely by
`VulkanRenderer::new` (`renderer/lifecycle.rs:43`); a pure-GLES run (`renderer: "gles"`) builds no
Vulkan renderer (`wire.entry/entry.rs:112,129`) and never reads it.** Therefore, in *this* code,
`infence` cannot affect a GLES run. If `infence` is observed to break GLES, it is either leftover from
an earlier state where the setting was wired more broadly (the vestigial `"kms"` `renderer_sync` value
— documented in `config.base` but read nowhere — and the currently-unused `gles.sync` crate both hint
it once was), or a side effect in the shared DrmCompositor/session setup. Pinning that down is part of
the separate "fix `infence`" task; it does not change the diagnosis above.

### This is NOT what option B addresses (and option B can't fix it)

Two orthogonal axes:
- **Option B = *where* compositing happens** — render card vs scanout card. It only runs when the
  render node's card ≠ the scanout card (`cross_device == true`); `present_option_b` is never called
  on a single card. So on single-card `renderD128` (render == scanout) option B is a no-op and cannot
  change the framerate.
- **The 30 fps = *how* the frame is synced** — the `device_wait_idle` above, identical on every
  Vulkan path.

If anything option B adds MORE synchronization on the split path (see below), so it is the wrong lever
for a framerate problem. There is no routing trick that reaches 60 fps; the only lever is async
fencing.

### The `infence` fast path is fully built and its prerequisites are present…

Verified in-tree, so when it works it genuinely engages (it does not silently fall back):
- render semaphore created exportable — `ExportSemaphoreCreateInfo::SYNC_FD`
  (`renderer/lifecycle.rs:226`);
- device enables `VK_KHR_external_semaphore_fd` (`vulkan.device/device.factory/factory.rs:21`);
- `export_sync_file` uses the `SYNC_FD` handle type (`vulkan.sync/sync.export/export.rs:54`);
- the udev path already calls `vk.set_drm_fd(display.drm_fd.clone())` (`wire.entry/entry.rs:135,217`),
  which `use_native_fence()` also requires (`native_fence_optin && drm_fd.is_some()`).

There is a self-healing fallback: if the driver rejects the export, one throttled warning
(`native KMS fence export failed … draining device`, once/min) and that frame reverts to
`device_wait_idle`.

### …but `infence` does not work on this hardware (currently hidden from the settings UI)

Empirically `infence` never produced a working 60 fps here, so it is hidden from all settings menus for
now and **fixing it is tracked as a separate task**. The default is deliberately NOT flipped
(`renderer_sync` stays unset = synchronous) — per the "no new defaults vs stable HEAD" rule, the async
path stays opt-in until it is proven on hardware.

### Propositions (fix `infence` → 60 fps Vulkan everywhere, single-card included)

Since the synchronous drain is the only alternative sync path, fixing `infence` is the actual fix for
the framerate across all configs. Likely failure modes, in order to check:

1. **Semaphore export rejected by the driver.** Watch for the throttled `native KMS fence export
   failed` warning / the `fence_fallback` stat. If it fires, `export_sync_file` on the binary
   `render_semaphore` is failing — check that the semaphore is *pending* (submitted) at export time,
   that i915/nouveau/nvidia advertise `SYNC_FD` semaphore export for this queue, and whether an
   `OPAQUE_FD` → `sync_file` conversion is needed instead.
2. **`sync_file` fd never becomes the atomic `IN_FENCE`.** Confirm the `SyncPoint::from(SyncFileFence)`
   actually reaches the DrmCompositor commit as an in-fence (via `honor_needs_sync` /
   `RenderFrameResult::needs_sync()`), rather than being dropped — if smithay isn't consuming it, KMS
   scans out without waiting and either tears or stalls.
3. **`drm_fd` not set on the renderer that presents.** `use_native_fence()` needs `drm_fd.is_some()`.
   The primary udev path sets it, but the option-B render-node `ctx.vulkan` and the scanout-card
   `ctx.vulkan_scanout` must each get `set_drm_fd` too, or they silently stay on the synchronous path.
4. **Frame-fence pacing regressions.** The native path pre-waits on `frame_fence` (submit.rs:60-67) to
   pace re-recording of its single reused command buffer; verify that fence is created signalled and
   reset each frame (it is, lifecycle.rs:34-39) and that a missed reset can't deadlock the wait.
5. **Diagnostic first:** add a one-shot log of the resolved sync mode + a frame-time histogram so the
   30↔60 transition is observable when toggling `renderer_sync`, instead of eyeballing fps.

### Fixing the Vulkan `infence` fence export (the actual blocker)

`infence` is unreliable across drivers, empirically:
- single-GPU main machine → **freeze**;
- Intel-iGPU-only machine → **artifacts / flicker**;
- this hybrid box → **works only when the render node is Intel** (Vulkan then hits 60 fps; NVIDIA render
  does not).

The scanout image barrier is correct (`command.record/record.rs:69-88` transitions to `GENERAL` and
releases to `VK_QUEUE_FAMILY_FOREIGN_EXT`), so this is NOT a missing-barrier artifact. The fragile part
is the **render-completion fence** handed to KMS. It is exported as a `sync_file` from a **binary
Vulkan semaphore** via `VK_KHR_external_semaphore_fd` / `SYNC_FD` (`vulkan.sync/sync.export/export.rs
::export_sync_file`, submit.rs:389). Semaphore→`SYNC_FD` export has driver-dependent semantics, and
that is exactly what varies:

- **Freeze** = the exported fd **never signals**. On old NVIDIA smithay sets `supports_fencing = false`
  (`vendor/…/compositor/mod.rs:1183-1206`: `IN_FENCE_FD` breaks the NVIDIA driver), so the fence is
  consumed by a CPU wait (`honor_needs_sync` → `SyncFileFence::wait`, a blocking `poll(-1)`); if the fd
  never signals, that poll blocks forever → hang. (Where `supports_fencing = true` and the fd never
  signals, the atomic commit's `IN_FENCE` never releases → no vblank → same hang.)
- **Artifacts** = the fd is **invalid or signals too early**, so KMS scans out before the render
  completes. `SyncFileFence::wait` currently treats `POLLNVAL`/`POLLERR` as *success* (returns `Ok`,
  only warns — `sync.fence/fence.syncfile/syncfile.rs`), so a bad fd becomes a fake instant wait →
  tearing instead of a clean fallback.
- **Works on Intel** = anv's semaphore `SYNC_FD` export happens to produce a correct
  render-completion fence there.

This is the concrete GLES-vs-Vulkan gap in one line: smithay's GLES fence is made *by smithay* and
known-valid; the Vulkan fence is hand-exported and its validity depends on driver semaphore-`SYNC_FD`
support.

**LANDED — a new gated `renderer_sync: "infence_2"` mode (compiles; `y5_compositor` links under
`backend-native,renderer-vulkan`; lint 0 failures).** The original `infence` path is untouched
(byte-identical). `infence_2` carries the robustness fixes below **except the timeout/probe changes**
(deliberately omitted per request):
- `renderer/mod.rs` — the `native_fence_optin: bool` is now a `SyncMode { Synchronous, InFence,
  InFenceV2 }` enum; `use_native_fence()`/`use_fence_v2()` derive from it.
- `renderer/lifecycle.rs` — parses `infence_2`; needs `VK_KHR_external_fence_fd` (else falls back to
  synchronous at init); creates `frame_fence` `SYNC_FD`-exportable in v2.
- `device.factory/factory.rs` — `external_fence_fd` added as an OPTIONAL device extension (enabled
  when advertised; never breaks device creation where absent).
- `sync.export/export.rs::export_fence_sync_file` — exports the VkFence as a `sync_file`; `Ok(Some)`
  = real fence, `Ok(None)` = the `-1` already-signalled sentinel, `Err` = failure; rejects other
  invalid fds.
- `sync.fence/…/syncfile.rs` — `SyncFileFence::new_strict` treats `POLLNVAL`/`POLLERR` as `Err` (no
  fake-success); the wait is NOT time-bounded (timeout omitted by request).
- `renderer/submit.rs` — v2 branch: export the VkFence, wrap strict, `-1` → signalled, and on any
  export failure fall back to `device_wait_idle()` (the known-good synchronous path).

Not implemented (excluded): the bounded/timeout wait and the startup fence-probe (both rely on
timeouts). Consequence: `infence_2` fixes the *fragility* (better fence source, no fake-waits, no
proceed-on-garbage, clean fallback on export failure) and gives NVIDIA a shot via the VkFence source,
but a driver that returns a *valid-but-never-signalling* fd can still block in `wait` (no timeout to
rescue it) — that case still needs the probe/timeout, which were held back.

**Fix plan (prioritized):**

Observed split (user): **Intel is graceful** — it either honours the fence or effectively no-ops it, so
it survives (works, or at worst flickers); **NVIDIA never renders anything** under `infence`. That
matches the model: on NVIDIA `supports_fencing = false`, so the fence is consumed by
`SyncFileFence::wait`'s blocking `poll(-1)`, and NVIDIA's binary-semaphore `SYNC_FD` never signals → the
poll blocks forever → the frame loop never advances → black. So the two levers are (a) never block
unboundedly on a fence, and (b) never even take the fence path on a device that can't produce a working
one.

1. **Safety floor — never proceed on, or hang on, a bad fence (turns freeze/artifacts into correct
   30 fps).**
   - `export_sync_file`: reject `raw < 0` instead of wrapping `-1` in an `OwnedFd`.
   - `SyncFileFence::wait`: on `POLLNVAL`/`POLLERR` return `Err`/`Interrupted`, never `Ok` — an invalid
     fd is a failed wait, not a completed one. **And bound the wait** — `poll` with a timeout of a few
     refresh intervals, not `-1`; a fence that hasn't signalled by then is treated as failed. This
     alone stops the NVIDIA hang.
   - `submit_frame`: if export fails, the fd is invalid, or the wait times out, fall back to
     `device_wait_idle()` + `SyncPoint::signaled()` for that frame (the known-good synchronous path).
     Result: on drivers where `SYNC_FD` export is broken, Vulkan degrades to a correct 30 fps instead
     of hanging (NVIDIA) or tearing (some Intel). This alone makes `infence` safe to expose again.
2. **Use a better-defined fence source.** Two options, both more portable than binary-semaphore
   `SYNC_FD`:
   - Export the **`VkFence`** the submit already signals (`self.frame_fence`) via
     `VK_KHR_external_fence_fd` `SYNC_FD`. Requires adding `ash::khr::external_fence_fd::NAME` to
     `device.factory/factory.rs::required_extensions` (currently only `external_semaphore_fd` is
     enabled) and creating `frame_fence` with `ExportFenceCreateInfo(SYNC_FD)`. Fence→`SYNC_FD` has
     cleaner "signals on completion" semantics than semaphore→`SYNC_FD`.
   - Or use the existing **DRM syncobj bridge** (`export.rs::bridge_to_syncobj`, already written) — the
     explicit-sync path KMS prefers (`DRM_CAP_SYNCOBJ`), which is how mutter/kwin drive this reliably.
3. **NVIDIA:** with (1)+(2) the CPU-wait-on-a-*valid*-fence path is brief (like GLES on NVIDIA, which
   also can't use `IN_FENCE` yet still hits 60 by waiting on its EGL fence), so NVIDIA render should
   reach 60. If the fence still can't be made reliable there, (1) keeps it at a correct 30 instead of
   freezing.
4. **Per-device probe → safe default.** At startup, submit a trivial command, export the fence, and
   `poll` it with a timeout; only enable `infence` for this device if it signals within budget. That
   converts `infence` from a global opt-in that hangs some machines into an auto-enabled, per-GPU-safe
   default — the prerequisite for re-exposing it in settings.
5. **Diagnostics:** log the resolved sync mode + a fence-signaled-vs-timeout counter + a frame-time
   histogram, so freeze/flicker/30-vs-60 are attributable instead of eyeballed.

Order to ship: (1) immediately — it removes the freeze/tearing and makes the setting safe; then (2)
for the real 60 fps on the drivers where `SYNC_FD`-from-semaphore is broken; then (4) to make it a
default and bring it back to the settings UI.

### Option-B split path — a second, smaller sync cost (even once `infence` works)

`present_option_b` composes on the render GPU into an offscreen, then does a per-frame CPU `sp.wait()`
on the render-completion fence before the scanout GPU imports it (the winit path does the same — it is
the correct way to sync two devices *without* a cross-device semaphore). So on the NVIDIA→Intel split,
`infence` removes the big `device_wait_idle` on the render GPU, but that handoff wait remains → the
split may land above 30 without fully reaching 60.

Proposition: replace the CPU `sp.wait()` with a **cross-device GPU-GPU wait** — export the render
semaphore as a `sync_file` and pass it as the scanout renderer's *wait* fence (or attach it to the
imported dmabuf's implicit sync), so neither CPU thread blocks. `vulkan.sync` already has the
export/import primitives; this is a focused follow-up, only needed if the split path is still not
smooth after `infence` is fixed.

## 6. Remaining work (for the hardware-in-the-loop session)
1. Frame-executor renderer-per-output (make true multi-monitor-multi-GPU actually render).
2. Per-surface `set_feedback` via `topology.feedback::advertise` (multi-primary modifier hints).
3. Secondary monitors in the rim output snapshots (settings UI).
4. Runtime GPU hotplug (optional).
5. Verify option-B cross-device present + secondary-card master/modeset on real hardware.
6. **Fix `infence` (async KMS fence) — the real fix for Vulkan 30 fps in every config (§5).**
7. Option-B cross-device GPU-GPU semaphore handoff to drop the per-frame `sp.wait()` (§5).

# The untiling blit: render→scanout handoff across a device boundary

Companion to `document/GPU_TOPOLOGY.md`. That doc maps *what* split render/scanout
is and which selection/assembly pieces have landed. This doc is about the **per-frame
pixel handoff** on a split system — specifically the **de-modifier ("untiling")
blit** that is the mandatory correctness floor, and the **zero-copy fast path** that
replaces it whenever the memory topology allows.

> Scope note: this is bridge **#2** (compositor render node → scanout card). It is the
> *same* tile→linear problem a PRIME-offloaded **client** app already solves for
> bridge #1 (client GPU → compositor) inside its own driver. The difference is that on
> `render_node = <the render GPU>` the compositor must do that handoff for its **own**
> composited output — no client driver is in the loop to do it for us.

## The core problem, in one paragraph

On a split system the compositor renders on the **render GPU** and must present on a
framebuffer owned by a **different** DRM device (the **scanout card**). The rendered
buffer carries the render GPU's preferred layout — a vendor-specific **tiled** DRM
format modifier (e.g. NVIDIA block-linear `0x03…`, AMD DCC, Intel Y/CCS). The scanout
card's importer (GLES/EGL on the scanout device, or its display planes) can only
consume a modifier **it** understands. When render and scanout are different vendors,
the intersection of "modifiers the render GPU can *produce as a color target*" and
"modifiers the scanout device can *import*" is frequently **empty** — not even
`LINEAR` is shared, because many GPUs (NVIDIA proprietary observed) do **not** expose
`LINEAR` as a color-attachment-capable dmabuf modifier. Result: `eglCreateImageKHR`
fails with `BAD_MATCH`, and there is no single buffer both devices can touch.

The fix is to stop trying to share one buffer and instead **convert**: render into the
render GPU's native tiled buffer (always works), then GPU-copy that into a second
buffer in a layout the scanout device *can* import. That copy is the untiling blit.

## The decision axis: unified vs separate memory

Whether the blit is needed at all is decided by **memory topology**, not by the node
split alone. `document/GPU_TOPOLOGY.md` names the two axes; this is the one that
governs the frame path.

### Zero-copy fast path — unified memory (UMA)

When the render GPU and the scanout engine **share one physical memory pool**, a dmabuf
the render GPU produced already lives where the scanout side can read it. The handoff
is a **re-import of the same fds**, no pixels move. This is the target for:

- **NVIDIA Jetson (Tegra/Orin), NVIDIA "nano".** One GPU + one DRAM shared with the
  display controller; render-only node (`nvgpu`) + display-only KMS card
  (`nvidia-drm`). Same vendor, shared memory → the scanout card can usually import the
  render GPU's tiled buffer directly (this is what the earlier `gpu_scanout_bridge`
  experiment leaned on — it forces a plane-scannable tiled modifier that, on UMA, the
  same-vendor display accepts).
- **AMD unified-memory / "AI" APU-class parts and other SoCs with a shared pool.**
  Integrated GPU + display block over one memory controller. A render dmabuf is
  scanout-visible without a copy.
- **Any single-GPU desktop** — the degenerate case where render card == scanout card,
  `route = None`, byte-identical buffer, trivially zero-copy.

On these, the untiling blit must be **skipped** — paying for a full-frame copy on
hardware that doesn't need one is a real regression (bandwidth, latency, power). The
frame path must *detect* the zero-copy opportunity and take it.

### Blit floor — separate memory

When render and scanout sit on **distinct physical memory** (discrete dGPU VRAM + iGPU
system RAM, or two discrete GPUs), crossing the boundary is a genuine copy over PCIe
(P2P if supported, else a bounce through system RAM). Even if the formats *did* line
up, the memory can't be shared. This is:

- **Discrete PRIME laptop** — Intel/AMD iGPU owns the eDP panel, NVIDIA/AMD dGPU
  renders. **(The configuration that motivated this doc: NVIDIA GTX 1050 render →
  Intel UHD 620 scanout, separate memory, cross-vendor, empty modifier
  intersection.)**
- **Reverse-PRIME / muxless** — a connector wired to the dGPU while compositing on the
  iGPU (or vice-versa).
- **Multi-GPU, monitor-per-GPU** — each foreign output is a separate-memory crossing.

On these the blit is **mandatory and unavoidable**. There is no format cleverness that
elides it; separate memory means a copy.

## The decision, made per (render, scanout) pair at runtime

Not compile-time, not global. For each output's handoff:

1. **Is this output's scanout device the render device (or UMA-equivalent)?** →
   `route = None` / zero-copy. Import the render dmabuf directly; done.
2. **Can the render-produced modifier be imported by the scanout side as-is?**
   (Negotiate the intersection of render-produced ∩ scanout-importable.) If non-empty →
   zero-copy re-import even across a node split (the UMA/same-vendor case). Done.
3. **Otherwise** → untiling blit: render tiled (native) → GPU-copy into a
   scanout-importable buffer (LINEAR is the universal floor) → hand the converted
   buffer to the scanout importer.

Step 2 is the optimization; step 3 is the floor. **Build the floor first** — it is
always-correct — then gate the fast path in front of it. A system that only ever blits
is *correct* everywhere and *optimal* nowhere; that is the acceptable first landing.

## Why LINEAR is the blit's target, and the one open capability question

`LINEAR` (`DRM_FORMAT_MOD_LINEAR`, modifier 0) is the universal interop layout — every
importer handles it. The blit's **output** buffer is therefore LINEAR.

The one hardware capability the blit depends on: the render GPU must accept a LINEAR
dmabuf as a **copy/transfer destination** (`TRANSFER_DST`). This is a *weaker*
requirement than the color-attachment support that force-linear *rendering* needs — and
which we observed NVIDIA does **not** provide (hence the empty intersection under the
full `COLOR_ATTACHMENT | SAMPLED | TRANSFER` usage set). Whether NVIDIA (and others)
expose LINEAR under `TRANSFER_DST` **alone** is the gating fact for the blit and should
be probed before/while building (see the plan below). If even `TRANSFER_DST`-LINEAR is
unavailable, the fallback-of-last-resort is a CPU readback+upload (always works, slow) —
but that is not expected to be needed.

---

# Implementation plan: the untiling blit

## What landed (iced/monitor bridge — the floor)

The floor is implemented for bridge #2's **iced** surface (the crossing that panicked
at `surface.rs:305` / `handle.rs:36`). It is gated behind a new experimental flag so
it can never regress the existing single-buffer path, and it degrades to that path on
any allocation failure rather than aborting.

**Gate.** `GpuFlags::UNTILE_BLIT` (`"gpu_untile_blit"` in `experimental.json`), added in
`compositor.developer/.../experimental.base/base.rs`. Off ⇒ today's behavior verbatim.

**Decision (`Backing::allocate`,
`compositor.extension/compositor.monitor/monitor.runtime/runtime.surface/surface.base/src/surface.rs`).**
Blit mode is taken only when *all* hold:
1. `gpu_untile_blit` is on,
2. a distinct scanout card is configured (`Environment.scanout_node` set and `!=`
   `render_node`) — the split signal, read from config rather than plumbed from
   `assemble.display`, since the surface subsystem never sees the DRM `CopyRoute`, and
3. the render∩scanout modifier intersection is **empty**
   (`negotiate::bridge_intersection_empty`, added alongside `bridge_modifiers`; it
   ignores the FORCE_*/NO_NEGOTIATE flags so the two-buffer choice hinges on hardware,
   not a modifier preference).

Otherwise the byte-identical single-buffer path runs (single GPU; UMA/same-vendor
split with a shared modifier; or the flag off).

**Two-buffer `Backing`.** `Backing` became an enum `Single(SingleBacking) |
Blit(BlitBacking)`. `BlitBacking` holds a render-GPU-native **tiled** target
(`allocate_dmabuf` implicit modifier → `import_dmabuf_to_wgpu`, the reliable wgpu
import) plus a **LINEAR** buffer on the scanout card (`allocate_linear_on(scanout)`,
new in `dmabuf_alloc.rs`) imported *twice*: into the scanout GLES (the sample source
the compositor scans out) and into the render GPU's wgpu as a **copy destination**
(`import_dmabuf_to_wgpu_transfer_dst`, new `COPY_DST`/`TransferDst` variant in
`wgpu_import.rs` — the weaker-than-color-attachment usage phase 0 is about). Field
order preserves the drop discipline (imports before allocations).

**The copy (phase 3).** `Backing::post_render_blit()` encodes
`copy_texture_to_texture(tiled → linear)` and submits on the same wgpu queue. It is
triggered from `IcedInstance::render` immediately after `runtime.render_into(view)`
(iced already submitted its frame, so same-queue ordering makes the copy see it).
Accessors route correctly per mode: `create_render_view` → tiled target;
`gles_texture()` / `dmabuf()` → LINEAR buffer.

**Allocating LINEAR on NVIDIA (hardware finding).** The LINEAR buffer must be
allocated via the **`GBM_BO_USE_LINEAR` usage flag** (`create_buffer_object` with
`RENDERING | LINEAR`), *not* the explicit-modifier path
(`create_buffer_object_with_modifiers2([LINEAR])`) — on the GTX 1050 the latter
fails with `EINVAL` ("CreateBo … Invalid argument"), which dropped the whole surface
to the single-buffer path and re-triggered the `BAD_MATCH` panic. The usage flag is
honored and yields `DRM_FORMAT_MOD_LINEAR`. If the render GPU still refuses LINEAR,
`BlitBacking::allocate` falls back to allocating the LINEAR buffer on the **scanout
card** (which renders, though the cross-device copy can shear) rather than degrading
to single-buffer — so a LINEAR-allocation failure never reintroduces the panic.

**Which node owns the LINEAR buffer (revised).** Both buffers are allocated on the
**render node**, NOT the scanout card. The first attempt put the LINEAR buffer on the
scanout card so the render GPU wrote it directly ("reliable PRIME direction"); on the
GTX 1050 → UHD 620 target that produced a **partial-stretch shear** — the signature of
a cross-device `vkCmdCopyImage` mismodeling the foreign buffer's row pitch (Vulkan
takes the destination pitch from the *other* card's gbm stride). Allocating the LINEAR
buffer on the render node makes the untile copy **same-device** (the driver models both
images' pitch self-consistently) and demotes the cross-device step to importing a plain
**LINEAR dmabuf** into the scanout GLES — the universal interop path. The scanout side
never scans the iced buffer out directly; it samples it while compositing into the
scanout card's own framebuffer, so a render-node-resident LINEAR sample source is fine.

Both `compositor_monitor_runtime_surface_base` and
`compositor_monitor_compositor_iced_base` build clean.

## Still open

- **Phase 0 (hardware probe)** — the design assumes (a) the render GPU accepts a
  foreign LINEAR dmabuf under `TRANSFER_DST`, and (b) the render→scanout PRIME write
  direction works. Both are asserted by construction here but only *verified* on real
  split hardware (the GTX 1050 → UHD 620 target). CPU readback+upload is the
  last-resort fallback if `TRANSFER_DST`-LINEAR turns out unavailable.
- **Phase 6 (bevy mirror)** — done. `BevySurface`
  (`compositor.support/support.bevy/.../surface.base/lib.rs`) now carries the same
  `Single | Blit` backing with render-node LINEAR placement, `allocate_linear_on` /
  `import_dmabuf_to_wgpu_transfer_dst` mirrored into the bevy alloc/import crates, and
  the field-access sites migrated to `render_texture()` / `sample_gles()` /
  `scanout_dmabuf()`. The render target is still the long-lived `Arc<wgpu::Texture>`
  cloned at `create_in_space` (now the tiled buffer in blit mode); the per-frame blit
  is triggered in `BevyInstance::tick()` immediately after `runtime.update()` (Bevy's
  render sub-app has submitted the frame by then). Builds clean.
- **Phase 7 (verify)** — needs the split host: confirm a LINEAR scanout buffer, no
  `BAD_MATCH`, and a UMA no-regression check (single-GPU path must stay byte-identical,
  which the gate guarantees).

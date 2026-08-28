# dmabuf feedback: before / after the advertise-narrowing

What `zwp_linux_dmabuf_v1` feedback y5 published to clients before and after the
change that intersects the EGL import set with what the **active compositing
renderer** can actually import (`negotiate.compositor::advertise`).

Measured on: NVIDIA RTX 4090, driver 595.80, `/dev/dri/renderD128`, single
DP-1 (Dell U4919DW) at 5120x1440, `COMPOSITOR_RENDERER=vulkan`, `hdr=false`.
Tool: `./dmabuf-probe` (this directory).

## Provenance — read this before trusting a number here

| Datum | Source | Verbatim? |
|---|---|---|
| POST fourccs + modifiers | `dmabuf-probe` run 2026-08-13 | **yes**, pasted below |
| PRE fourcc list (48) | reconstructed from the compositor's own `register_dmabuf` log line, which prints the offered **and** withheld sets: 17 + 31 = 48 | **yes**, exact — the union is what was advertised before narrowing |
| PRE modifiers/fourcc (14 = 12 vendor + LINEAR + INVALID) | a count recorded in conversation while the pre-change build was live; **the probe output itself was not retained** | **no** — treat the 12 as unconfirmed, see [Open question](#open-question) |

The pre-change probe cannot be reproduced without rebuilding with the narrowing
reverted. Everything below marked verbatim can be re-measured at any time.

## PRE — EGL set, unnarrowed

48 fourccs. This is `EGLDisplay`'s `texture_formats` for the GBM/NVIDIA platform,
passed through to feedback unchanged. Per-fourcc modifier count recorded as 14
(vendor block-linear + `LINEAR` + `DRM_FORMAT_MOD_INVALID`) → on the order of
~670 pairs.

The 48, split by what the change later did with them:

**Kept (17)**
```
AB24 AB30 AR15 AR24 AR30 BA15 BG16 BX15 R8   RA15 RG16 RX15 XB24 XB30 XR15 XR24 XR30
```

**Withheld (31)**
```
AB12 AB15 AB4H AR12 BA12 BA24 BA30 BG24 BX24 BX30 NV12 NV16 NV21 NV24 P010 P012
P210 R16  RA12 RA24 RA30 RG24 RG32 RG88 RGB8 RX24 RX30 UYVY XB15 XB4H YU12
```

Only 8 of the 48 were ever observed to actually draw. `XB4H`/`AB4H` produced the
`unsupported fourcc for the vulkan path` warning per frame — advertised by mesa's
EGL list, unmappable by the Vulkan import path — which is the bug that started
this work.

## POST — intersected with the Vulkan renderer's importable set

Verbatim, 2026-08-13:

```
./dmabuf-probe
global: zwlr_layer_shell_v1 (v5)
zwp_linux_dmabuf_v1 version 5
main device: 226:128
format table: 119 (format x modifier) pair(s)

-- tranche 0 (device 226:128)
   flags: 0x0
   119 index/indices
     AB24   7 modifier(s): NVIDIA(0x0300000000606010) NVIDIA(0x0300000000606011) NVIDIA(0x0300000000606012) NVIDIA(0x0300000000606013) NVIDIA(0x0300000000606014) NVIDIA(0x0300000000606015) LINEAR(0x0000000000000000)
     XB24   7 modifier(s): ... (same 7)
     R8     7 modifier(s): ... (same 7)
     XR15   7 modifier(s): ... (same 7)
     RX15   7 modifier(s): ... (same 7)
     BX15   7 modifier(s): ... (same 7)
     AR15   7 modifier(s): ... (same 7)
     RA15   7 modifier(s): ... (same 7)
     BA15   7 modifier(s): ... (same 7)
     AR24   7 modifier(s): ... (same 7)
     XR24   7 modifier(s): ... (same 7)
     RG16   7 modifier(s): ... (same 7)
     BG16   7 modifier(s): ... (same 7)
     AR30   7 modifier(s): ... (same 7)
     XR30   7 modifier(s): ... (same 7)
     XB30   7 modifier(s): ... (same 7)
     AB30   7 modifier(s): ... (same 7)
```

Every one of the 17 carries the identical 7: `0x0300000000606010`–`…015` plus
`LINEAR`. 17 x 7 = 119, matching the advertised table size. The compositor's own
log agrees: `advertising 119 pair(s) over 17 fourcc(s)`.

## Delta

| | PRE | POST |
|---|---|---|
| fourccs | 48 | 17 |
| modifiers / fourcc | 14 (recorded) | **7** (verbatim) |
| pairs | ~672 | **119** |
| `DRM_FORMAT_MOD_INVALID` offered | yes | **no** |
| fourccs that actually draw | 8 of 48 | 17 of 17 |

### Why each fourcc left

- **Colour policy** (`expressible()`): `XB4H`, `AB4H` — fp16 linear-light. The SDR
  composite cannot say what those numbers mean, so a linear 0.21 would be emitted
  as encoded 0.21 (~0.03 of intended light). Withheld deliberately; they return
  when `hdr=true` ships. See `COLOR_TIER.md` Tier 3.
- **Device says no** (`import_formats()` found no sampleable modifier): `BA12`,
  `RG24`, `R16`, and the rest of the 12-bit / packed / RGB8 group.
- **Multi-plane YUV**: `NV12`, `NV21`, `NV16`, `NV24`, `P010`, `P012`, `P210`,
  `YU12`, `UYVY` — no ycbcr sampler and no in-shader plane conversion yet.
  `COLOR_TIER.md` Tier 4 records the two routes back.

### Why INVALID left — structural, not a policy choice

`vendor/smithay/src/backend/egl/display.rs:995` inserts `Modifier::Invalid` for
**every** fourcc, unconditionally. Vulkan's `VK_EXT_image_drm_format_modifier`
has no notion of `DRM_FORMAT_MOD_INVALID`, so
`get_format_modifier_properties()` can never report one, so every
`(fourcc, INVALID)` pair necessarily falls out of an exact-pair intersection.

This is honest rather than lossy: `memory.import::import()` always builds
`ImageDrmFormatModifierExplicitCreateInfoEXT` with a concrete modifier, so the
Vulkan path could never have imported an implicit-modifier buffer anyway. But it
**is** a behaviour change for v3 clients (no feedback, so they read the
`format`/`modifier` event list) and for the "empty `bridge_modifiers()` → use the
implicit path" fallback our own producers document.

## Implicit modifiers on the Vulkan path

### What "explicit create info" actually costs us

`memory.import/import.base/import.rs:96` builds, for every client buffer:

```rust
vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
    .drm_format_modifier(Into::<u64>::into(modifier))
    .plane_layouts(&plane_layouts)   // offset + row_pitch per plane, from the dmabuf
```

"Explicit" means *the caller states the memory layout and Vulkan believes it*. The
driver does not inspect the fd; it takes `(modifier, offsets, strides)` as ground
truth and builds an image view over that. So the modifier is a **required input**,
not something the import can discover.

`DRM_FORMAT_MOD_INVALID` is not a layout — it means "ask the driver what its
default layout for this usage is", a question only the allocating driver can
answer. There is **no Vulkan entry point that recovers a dmabuf's implicit
layout.** `vkGetImageDrmFormatModifierPropertiesEXT` reports the modifier of an
image *you already created*; it cannot tell you about a foreign buffer.

Current behaviour with an implicit buffer: nothing in `kernel.vulkan` tests for
`Modifier::Invalid`, and the `may_import()` gate is fourcc-level only, so
`0x00ffffffffffffff` is passed straight to `vkCreateImage`, which fails (the
modifier is not among those the format supports). It fails late, as a driver
error, rather than as a legible refusal.

### The trap to avoid

`memory.export/export.base/export.rs:62` already uses
`ImageDrmFormatModifierListCreateInfoEXT` — hand Vulkan a list, let the driver
pick, then `get_image_drm_format_modifier_properties()` to learn the choice. The
ash extension handle is already wired. It is tempting to reuse that for import.

**Do not.** That pattern is for *allocation*, where the driver owns the memory and
so may choose freely. On import the memory already exists with a fixed layout; the
driver's pick is arbitrary and unrelated to it. Binding foreign memory to an image
whose chosen layout disagrees with the buffer gives garbage pixels at best and a
GPU page fault at worst — i.e. the Xid 31 class of failure, but by construction
instead of by race.

### Route A — resolve the modifier through GBM, then use the existing explicit path

The only sound approach: learn the real modifier out-of-band from something that
*does* know the driver's implicit rules, then feed the existing explicit path.
GBM knows, because it is the same driver that allocated the buffer.

1. `gbm_bo_import(GBM_BO_IMPORT_FD, {fd, width, height, stride, format})` — note
   this struct has **no** modifier field; the driver applies its own implicit rules.
2. `gbm_bo_get_modifier(bo)` → the concrete modifier.
3. Check it against `format::modifier::import_formats()` (sampleable set); refuse
   if absent.
4. Import as today, with that modifier substituted for `INVALID`.

Plumbing is cheap: `GbmDevice<DrmDeviceFd>` is already a field at
`native.assemble/assemble.display/display.base/display.rs:32`, the same layer that
constructs the Vulkan renderer in `native.wire/wire.entry`. No cross-layer
dependency is added.

Two constraints to verify before building it, neither assumed here:

- **Single-plane only.** `GBM_BO_IMPORT_FD` takes one fd and one stride.
  Multi-plane implicit buffers cannot round-trip (`GBM_BO_IMPORT_FD_MODIFIER`
  handles multi-plane but *requires* the modifier we are trying to discover). In
  practice implicit buffers are single-plane RGB (wl_drm, v3 clients), so this
  covers the real cases — but it is a hard ceiling, not an optimisation gap.
- **Driver-dependent.** On Mesa, `gbm_bo_get_modifier` returns a real modifier for
  an `IMPORT_FD` bo. On the NVIDIA proprietary GBM this is unmeasured; it may
  return `INVALID` and leave us where we started. **Measure before designing
  around it.** Also confirm what smithay's `GbmDevice` wrapper exposes rather than
  assuming a 1:1 mapping to the C API.

### Route B — decline, but decline early and legibly

Cheapest, and defensible: reject `Modifier::Invalid` at the top of `import_dmabuf`
with a named error, next to the `may_import()` gate. Turns a late opaque
`vkCreateImage` failure into one line naming the cause. Since INVALID is no longer
advertised, a well-behaved client should not reach it at all.

This is the honest baseline; Route A is the upgrade.

### Route C — GLES fallback for implicit buffers only. Rejected.

The GLES renderer imports implicit buffers fine (EGL resolves the layout
internally). Routing only implicit buffers there would mean sharing a texture
across two renderers with different devices and allocators — far more machinery
than Route A, for a case Route A already covers.

### Who actually sends implicit buffers

- **wl_drm clients** — already unsupported on this path, independent of the
  narrowing: `renderer/import.rs:401` returns
  `Unimplemented("import_egl_buffer (wl_drm/EGL)")` and `egl_reader()` returns
  `None`, so the `ImportAll` dispatcher never routes them here. No regression.
- **`zwp_linux_dmabuf_v1` v3 clients** — no feedback, so they read the
  `format`/`modifier` event list. Since we no longer list INVALID they should pick
  an explicit modifier. **Unverified:** whether smithay validates the modifier (not
  just the fourcc) at `params.create`, and therefore whether a v3 client that sends
  INVALID anyway gets a graceful `failed` event or is killed by protocol error 4.
  Check `vendor/smithay/src/wayland/dmabuf/mod.rs` before assuming it is graceful.
- **Our own off-thread producers** — the live landmine.
  `negotiate.compositor::worker_modifiers()` documents an empty result as "the
  implicit path", and returns empty both when the intersection is empty and when
  the compositor has not published yet. A producer taking that branch now
  allocates a buffer the compositor cannot import. It should allocate LINEAR
  explicitly, or refuse, rather than fall back to implicit. Tracked in
  `COLOR_TIER.md` alongside the `Backend::importable()` data-dependency item.

## Open question — SETTLED, the answer is 6

**Resolved 2026-08-13 with `color.format.stress/vkfmt`**, which asks the device
directly (`vkGetPhysicalDeviceFormatProperties2` + `VkDrmFormatModifierPropertiesListEXT`,
no compositor, no surface). Every format tested reports exactly **7** modifiers —
the 6 vendor block-linear `0x0300000000606010`–`…015` plus `LINEAR` — of which 6
are color-attachable and all 7 sampleable:

```
XB30/AB30  A2B10G10R10_UNORM_PACK32   renderable()=YES  sampleable()=YES  -> 6 CA, 7 SA
XR30/AR30  A2R10G10B10_UNORM_PACK32   renderable()=YES  sampleable()=YES  -> 6 CA, 7 SA
XR24/AR24  B8G8R8A8_UNORM             renderable()=YES  sampleable()=YES  -> 6 CA, 7 SA
XB24/AB24  R8G8B8A8_UNORM             renderable()=YES  sampleable()=YES  -> 6 CA, 7 SA
XB4H/AB4H  R16G16B16A16_SFLOAT        renderable()=YES  sampleable()=YES  -> 6 CA, 7 SA
```

So the POST count of 7 per fourcc is the device's own answer and **no tiled
modifier was lost to the intersection**. The earlier "12" was a miscount, not an
EGL-only surplus: the KMS plane independently reports the same six
(`Unrecognized(216172782120099856)`–`(…861)`), so three sources agree on six.

Two facts worth keeping from that run: `LINEAR` is `SAMPLED`-only and never
color-attachable on this driver — which is why GBM `RENDERING|LINEAR` allocation
fails here — and the device renders **both** 10-bit channel orders, so any
10-bit asymmetry is the plane's, not the GPU's (see `COLOR_TIER.md` → the output
axis).

The paragraph below is kept as the original reasoning; the conclusion above
supersedes it.

The recorded PRE count was **12** vendor modifiers; POST shows **6**
(`…010`–`…015`). If the 12 was right, the Vulkan sampleable set is half the EGL
one and the missing 6 are EGL-only.

Corroborating evidence for 6 being the real hardware set: the scanout plane
advertises exactly those same 6 (logged as `Unrecognized(216172782120099856)`–
`(216172782120099861)` = `0x…606010`–`0x…606015`) plus `Invalid` and `Linear`,
i.e. 8. So 6 is the count the KMS plane and the Vulkan device independently
agree on — which makes a 12-entry EGL list plausible (EGL may expose sampling-only
block-linear variants) but also makes a miscount of the earlier probe plausible.

**Not a regression either way**: producers still negotiate tiled, not LINEAR —
`gpu alloc: iced surface … -> AR24 tiled (Unrecognized(216172782120099860)),
negotiated from 6 candidate(s)`, and likewise for the background worker. Nothing
was forced to a linear layout.

To settle it, revert the intersection in `negotiate.compositor::advertise` (return
`egl` unnarrowed), rebuild, re-probe, and diff the modifier list per fourcc.

## Observability gap this exposed

`register_dmabuf` logs fourccs *withheld entirely*, so losing a whole format is
visible. A format that survives but loses **modifiers** is invisible — exactly the
14→7 above, which had to be caught by hand with the probe. Worth adding the
per-fourcc modifier count to that log line.

## Re-measuring

```bash
cd compositor.developer/developer.tool/developer.tool.color/color.format.stress
make
WAYLAND_DISPLAY=wayland-N ./dmabuf-probe        # N = the y5 socket, see the startup log
```

Point it at a **nested** y5 you can kill, not your live session — see
`README.md`; the accept/refuse and draw tools in this directory have wedged the
host compositor and induced GPU device-loss.

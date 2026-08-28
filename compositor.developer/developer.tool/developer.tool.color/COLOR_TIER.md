# Colour / format support tiers

Parked work, written down so it can be picked up cold. Two axes, and keeping them
apart matters — conflating them hid a bug for the lifetime of the feature:

* **input** — which DRM formats can the Vulkan path *import* from a client, and what
  does each one cost? The tiers below, and the advertise/import guarantee.
* **output** — which format do we hand to KMS, and did we get the one we asked for?
  See [The output axis](#the-output-axis--what-we-hand-to-kms).

Status: **blocked behind the `Xid 31` lifetime bug** (GPU reading unmapped memory →
`VK_ERROR_DEVICE_LOST` → session freeze; see the end of this file). None of the work
below should start until that is fixed, because the test harness that validates it is
also the thing that reliably triggers the crash.

Everything here is measured on one machine — NVIDIA 595.80, open kernel module,
`/dev/dri/renderD128` — with
`compositor.developer/developer.tool/developer.tool.color/color.format.stress`.

## Guarantee: nothing outside Tier 0 is advertised or importable

Audited and closed, so the parked tiers cannot leak in through a side path. Four gates,
all reading the same two answers (`table::classify` for capability,
`negotiate.compositor::expressible` for policy):

| gate | what it stops | code |
|---|---|---|
| advertisement | the fourcc never reaches a client's feedback | `negotiate.compositor::advertise` (policy) ∩ `modifier::import_formats` (device) |
| buffer creation | `params.create` → smithay posts `InvalidFormat` (error 4) | `negotiate.compositor::importable_code`, via `wire.rs::dmabuf_import` |
| producer negotiation | an off-thread worker cannot pick one either | `negotiate.compositor::worker_modifiers` |
| draw-path import | refused even if a buffer arrives anyway | `renderer::import_dmabuf` → `may_import`, plus `vk_format` = `None` |

Per tier, the reason each is unreachable **on the Vulkan path**:

* **Tiers 1, 2, 4** — `classify` returns `Vk::None`/`Vk::Chroma`, so `vk_format()` is
  `None` and all eight call sites refuse (`.ok_or(UnsupportedFormat)`); `table::images()`
  skips them, so they can never enter `import_formats`, hence never the advertised set.
  Airtight by construction — a new variant is a compile error until classified.
* **Tier 3 (fp16)** — the one that needed work. `vk_format()` legitimately returns
  `Some`, so capability alone would have allowed an import. It is now refused by policy
  at the draw path (`may_import`) and for producers (`worker_modifiers`), matching the
  advertisement and creation gates that already withheld it.

Scope: this is the **Vulkan** path, and it is the **input** direction only. Nothing in
the table above constrains what the compositor hands to KMS — see the next section.
On the GLES path Tier 1/2 formats really are importable (EGLImage doesn't care about
channel order), and advertising them there is correct — `advertise` only narrows to the
active renderer's set.

## The output axis — what we hand to KMS

Added after this document's own Tier 0 claim turned out to be true and misleading at the
same time. Tier 0 lists `Argb2101010 Xrgb2101010 Abgr2101010 Xbgr2101010` as verified
byte-exact, and they are — as **client buffers we import**. On the axis nobody was
looking at, deep colour had never once engaged on the measuring machine.

### What was wrong

`scanout.surface/surface.output/output.base::color_formats(ten_bit)` offered
`[Xrgb2101010, Argb2101010, Argb8888, Abgr8888]` — R-first 10-bit only. The NVIDIA
primary plane offers, at 10 bits, only `AB30`/`XB30`:

```
plane accepts DrmFourcc(AB30): Invalid, <6 x NVIDIA block-linear>, Linear
plane accepts DrmFourcc(XB30): ...
plane accepts DrmFourcc(AR15): ...      <- R-first exists at 15 bits
plane accepts DrmFourcc(AR24): ...      <- and at 24
                                        <- but there is no AR30 / XR30
```

22 fourccs x 8 modifiers = the 176 pairs the startup log reports. R-first is present at
every depth *except* 10-bit, which is what makes "R-first at 10-bit too" such a natural
and wrong inference.

So both 10-bit rungs missed, and the session fell to `AR24` at 8-bit.

### Why it hid for so long

Three independent reasons, all worth remembering because they generalise:

1. **The failure is shaped exactly like the design working.** `color_formats` is a
   *ladder*; smithay is supposed to walk it and take the first rung the plane accepts.
   `WARN Preferred format XR30 not available: NoSupportedPlaneFormat` is the designed
   output of a healthy probe. Nothing anywhere compared *requested* depth against
   *achieved*.
2. **The fallback is coherent, not broken.** `scanout_is_deep()` deliberately reads the
   achieved fourcc, so once it returned false every producer consistently chose 8-bit
   (`background worker: rendering DrmFourcc(AR24) (session deep=false)`). An internally
   consistent 8-bit session. No artifact. The only symptom is banding on a large
   gradient, which you would blame on the shader.
3. **The producer audit checked the rung and not the gate.** The audit below concluded
   "no producer picks an unsupported fourcc — everything is hardcoded `Argb8888` except
   the background worker's checked `[Argb2101010, Argb8888]` ladder." The rungs *were*
   checked (`usable()` tests `renderable()` and a non-empty modifier list). Nobody asked
   whether the 10-bit rung's gate — `scanout_is_deep()` — could ever open. It could not.

### The device is symmetric; only the plane is not

Measured with `color.format.stress/vkfmt` (new — queries `VkFormat` capability and the
per-modifier tiling features with no surface, no compositor, so it cannot wedge a
session the way `dmabuf-draw` can):

| DRM | VkFormat | renderable | sampleable | modifiers |
|---|---|---|---|---|
| `XB30`/`AB30` | `A2B10G10R10_UNORM_PACK32` | YES | YES | 6 color-attachable, 7 sampleable |
| `XR30`/`AR30` | `A2R10G10B10_UNORM_PACK32` | YES | YES | 6 color-attachable, 7 sampleable |
| `XR24`/`AR24` | `B8G8R8A8_UNORM` | YES | YES | 6 / 7 |
| `XB24`/`AB24` | `R8G8B8A8_UNORM` | YES | YES | 6 / 7 |
| `XB4H`/`AB4H` | `R16G16B16A16_SFLOAT` | YES | YES | 6 / 7 |

So the Vulkan device renders and samples **both** channel orders equally. The asymmetry
is entirely in the KMS plane. Two incidental confirmations: the vendor modifier count is
**6**, not the 12 recorded earlier (see `probe-change.MD`'s open question — this settles
it), and `LINEAR` is `SAMPLED` only, never color-attachable, which is why GBM
`RENDERING|LINEAR` allocation fails on this driver.

### Fixed

* `color_formats` now leads with `Xbgr2101010, Abgr2101010`, keeps `Xrgb2101010,
  Argb2101010` as a fallback for hardware exposing only those, and leaves the 8-bit tail
  alone. B-first is also the better-supported order across this tree: smithay's GLES
  tables map only `Abgr2101010` (R-first 10-bit has no GL mapping at all) and
  `negotiate.wgpu` omits `Argb2101010` for want of a wgpu `TextureFormat`.
* `negotiate.compositor::scanout_fourcc()` — new accessor, exposing the achieved fourcc
  and not just `scanout_is_deep()`, so a producer can follow the session's channel order
  instead of guessing it.
* The background worker's ladder is no longer a hardcoded order: it tries the alpha
  sibling of the session's own fourcc first, the other order second, then the 8-bit
  floor. Both orders are renderable here, so this is about not diverging from the
  scanout for no reason — where the buffer can be promoted straight to a plane, only the
  order the plane exposes can take that path.

**Verified on hardware 2026-08-13.** No `NoSupportedPlaneFormat` warning; the session
came up at 10 bits for the first time on this machine and the worker followed the
session's channel order:

```
native scanout: hdr=false deep_color=true → 10-bit=true
scanout swapchain: fourcc=DrmFourcc(XB30) modifier=Invalid (7 offered: …)
background worker: rendering DrmFourcc(AB30) (session deep=true, scanout=Some(DrmFourcc(XB30)))
gpu alloc: background worker -> DrmFourcc(AB30) tiled (Unrecognized(216172782120099860)), negotiated from 7 candidate(s)
```

### The guarantee this axis still lacks

The input axis has a four-gate guarantee. The output axis has none, and the missing one
is precisely what would have caught this:

> **deep colour requested ⇒ achieved, or explicitly reported as not achieved.**

Today a missed rung is indistinguishable from a healthy probe. Cheapest form: after the
swapchain settles, compare the achieved fourcc against the request and `warn!` when a
depth was asked for and not obtained, naming the plane's actual 10-bit codes. The
`set_scanout_fourcc` call site already has both halves of the comparison in hand.

Related and still open: `register_dmabuf` logs fourccs withheld *entirely*, so losing a
format is visible, but a format that survives while losing *modifiers* is not — the
14→7 narrowing in `probe-change.MD` had to be caught by hand with `dmabuf-probe`.

### Where the output decisions live

| what | file |
|---|---|
| the offered scanout ladder | `kernel.scanout/scanout.surface/surface.output/output.base/output.rs` (`color_formats`) |
| requested depth → `ten_bit` | `kernel.native/native.assemble/assemble.renderer/renderer.base` |
| plane's real format list (logged at startup) | `kernel.native/native.assemble/assemble.display/display.base` (`log_plane_formats`) |
| achieved fourcc, published once | `negotiate.compositor::set_scanout_fourcc` / `scanout_fourcc` / `scanout_is_deep` |
| the background's own ladder | `compositor.background/background.two/two.worker/worker.format` |
| device capability probe | `developer.tool.color/color.format.stress/vkfmt.c` |

## Where the decisions live

| what | file |
|---|---|
| the exhaustive fourcc table (no wildcard arm) | `kernel.vulkan/vulkan.format/format.table/table.base/table.rs` |
| `vk_format` / `opaque` / `refusal` accessors | `kernel.vulkan/vulkan.format/format.query/query.base/query.rs` |
| device filter → the advertised set | `kernel.vulkan/vulkan.format/format.modifier/modifier.base/modifier.rs` (`import_formats`) |
| view creation (where a swizzle would go) | `kernel.vulkan/vulkan.memory/memory.import/import.base/import.rs` |
| advertise / accept policy | `kernel.graphic/graphic.bridge/bridge.negotiate/negotiate.compositor/compositor.rs` |

Naming rules that make the tables below decidable: **DRM** names packed formats
MSB→LSB of a little-endian word (`ARGB8888` = `[31:24]A [23:16]R [15:8]G [7:0]B`, so
memory reads `B,G,R,A`). **Vulkan** `*_PACK16/32` names the packed word the same way,
but unpacked names (`R8G8B8A8_UNORM`) are **memory order**. A `VkComponentMapping`
entry `r: G` means "deliver the image's G component as the shader's r".

---

## Tier 0 — already supported

Direct layout matches, live today and verified byte-exact on hardware:
`Argb8888` `Xrgb8888` `Abgr8888` `Xbgr8888` `Argb2101010` `Xrgb2101010`
`Abgr2101010` `Xbgr2101010` `Rgb565` `Xrgb1555` `Bgra4444` `Rgb888` `Bgr888`
`R8` `R16` `Gr88` `Gr1616`.

Measured post-fix: the 8888 quartet and both 2101010 returned the authored greys
exactly (`64/128/192`) with pure R/G/B/W patches; `Rgb565` and `Xrgb1555` returned
`66/132/198`, which is correct 5-6-5 and 5-5-5 quantisation, not error; `R8` returned
`64/128/192` in red with black G/B patches, as a single-channel format must.

`Bgra4444`, `Rgb888` and `R16` are mapped but **withheld on this GPU**: NVIDIA reports
no sampleable DRM modifier for `B4G4R4A4_UNORM_PACK16`, `B8G8R8_UNORM` or
`R16_UNORM`, so `import_formats` drops them. The table proposes, the device disposes —
they should light up unchanged on a GPU that supports them.

---

## Tier 1 — channel order only: supportable with a view swizzle

These are **not** missing from Vulkan. They are permutations of formats Vulkan has,
and `VkComponentMapping` on the image view expresses exactly a permutation. The
mechanism is already in the import path — it sets `components` today to force
`a: ONE` for `X` formats — so this is a fuller table, not a new concept.

| DRM fourcc | memory / field order | import as | components |
|---|---|---|---|
| `Bgra8888` | bytes `A,R,G,B` | `R8G8B8A8_UNORM` | `r:G g:B b:A a:R` |
| `Bgrx8888` | bytes `X,R,G,B` | `R8G8B8A8_UNORM` | `r:G g:B b:A a:ONE` |
| `Rgba8888` | bytes `A,B,G,R` | `R8G8B8A8_UNORM` | `r:A g:B b:G a:R` |
| `Rgbx8888` | bytes `X,B,G,R` | `R8G8B8A8_UNORM` | `r:A g:B b:G a:ONE` |
| `Rg88` | bytes `G,R` | `R8G8_UNORM` | `r:G g:R b:ZERO a:ONE` |
| `Rg1616` | 16-bit `G,R` | `R16G16_UNORM` | `r:G g:R b:ZERO a:ONE` |
| `Argb4444` | `[15:12]A R G B` | `R4G4B4A4_UNORM_PACK16` | `r:G g:B b:A a:R` |
| `Xrgb4444` | `[15:12]X R G B` | `R4G4B4A4_UNORM_PACK16` | `r:G g:B b:A a:ONE` |
| `Abgr4444` | `[15:12]A B G R` | `R4G4B4A4_UNORM_PACK16` | `r:A g:B b:G a:R` |
| `Xbgr4444` | `[15:12]X B G R` | `R4G4B4A4_UNORM_PACK16` | `r:A g:B b:G a:ONE` |
| `Abgr1555` | `[15]A [14:10]B G R` | `A1R5G5B5_UNORM_PACK16` | `r:B g:G b:R a:A` |
| `Xbgr1555` | `[15]X [14:10]B G R` | `A1R5G5B5_UNORM_PACK16` | `r:B g:G b:R a:ONE` |
| `Argb16161616f` | halves `B,G,R,A` | `R16G16B16A16_SFLOAT` | `r:B g:G b:R a:A` † |
| `Xrgb16161616f` | halves `B,G,R,X` | `R16G16B16A16_SFLOAT` | `r:B g:G b:R a:ONE` † |

† also subject to the Tier 3 colour policy — the swizzle fixes the *order*, not the
transfer function.

**The caveat that must not be lost.** The fourcc never reaches Vulkan; we are
reinterpreting the memory as a different VkFormat. That is sound for `LINEAR`, and for
vendor tilings **at equal bits-per-pixel** (tiling depends on bytes per pixel, not on
channel order) — which is what mesa and wlroots already rely on. Two consequences:
`get_format_modifier_properties` must be queried for the **VkFormat actually created**,
not for a notional one; and a reinterpretation must never change bpp.

**Implementation sketch.** Add the mapping to `Vk::Image` in `table.rs` (the crate
already depends on `ash`, so it can name `vk::ComponentSwizzle`); expose
`query::components(fourcc) -> vk::ComponentMapping` that folds the existing `opaque`
rule into it; have `memory.import` build the view from that instead of its local
`a: ONE` decision. `import_formats` and the advertise/accept policy need no change —
they already walk the table. The test harness needs a pixel encoder per new fourcc in
`dmabuf-draw.c` before any of it can be verified visually.

---

## Tier 2 — genuinely unsupportable

A swizzle permutes components; it cannot relocate bit fields. These need a VkFormat
that does not exist:

| DRM fourcc | why |
|---|---|
| `Rgba1010102` `Rgbx1010102` `Bgra1010102` `Bgrx1010102` | alpha in the **low** 2 bits; every Vulkan 10-bit packed format puts it in the top 2 |
| `Rgb332` `Bgr233` | 3-3-2 has no Vulkan analogue at any bit depth |
| `Axbxgxrx106106106106` | no equivalent layout |
| `*_a8` two-plane (`Rgb565_a8`, `Xrgb8888_a8`, …) | RGB plus a separate A8 plane; Vulkan has no such image |
| `C8` | paletted; needs palette plumbing that does not exist |

Correctly refused today, and the refusal reason already reaches the log via
`query::refusal`.

---

## Tier 3 — fp16, and it does **not** need HDR

`Abgr16161616f` / `Xbgr16161616f` (and the ARGB pair once Tier 1 lands) import fine.
What is missing is not the display and not the format — it is that the SDR composite
has no transfer awareness. fp16 carries **linear-light, extended-range** values; the
SDR path passes sampled colour through as if it were sRGB-encoded, so a linear `0.21`
mid-grey is emitted as encoded `0.21` and lands at ~`0.03` of the intended light: far
too dark, with everything above 1.0 clipped. That is why they are withheld while
`color_managed()` is false, and why the gate currently keys on HDR — the HDR composite
(`vulkan.pipeline/pipeline.hdr/hdr.base/shaders/composite_hdr.wgsl`) is the only shader
in the tree with a per-surface `transfer` input (`0 sRGB, 1 PQ, 2 HLG, 3 linear`).

Two ways to support it in SDR, both without HDR hardware:

1. **Convert at import (recommended).** A small blit pipeline: sample the fp16 image,
   apply the sRGB OETF, clamp, write an 8/10-bit UNORM texture the rest of the pipeline
   already handles correctly. Contained in the Vulkan renderer, does not touch the
   shader every other surface goes through, costs one pass per buffer update.
2. **A per-surface flag in the SDR shader**, mirroring the HDR one. Cheaper at runtime,
   but edits the hot path everything else uses.

Either makes `set_color_managed(true)` honest in SDR. The policy predicate reads from
one place (`negotiate.compositor::expressible`), so nothing else changes — and an fp16
client that declares nothing still defaults to `transfer = 0` and will look wrong, but
that is the client failing to use `wp_color_management_v1`, not us.

---

## Tier 4 — planar Y/Cb/Cr, and the trade-off that was deliberately reversed

`Nv12` `Nv21` `Nv16` `Nv24` `P010` `P012` `P210` `Yuv420` `Yuyv` `Uyvy` … all classify
as `Vk::Chroma`. Vulkan can describe them only through
`VK_KHR_sampler_ycbcr_conversion` with a conversion-enabled sampler, which the renderer
does not build.

The driver *does* advertise them: `dmabuf-probe` on this machine lists `NV12 NV16 NV21
NV24 P010 P012 P210 UYVY YU12` among 48 fourccs, so this is a real capability gap, not
a theoretical one.

### This was a deliberate choice once, in the other direction

`y5.graphic/graphic.display/display.output/output.rs` used to carry this, and it is the
reason the wide EGL set was published even under Vulkan:

> The FORMATS are the GLES EGL import set, DELIBERATELY, even when the compositor goes
> on to composite with Vulkan. It is tempting to republish the Vulkan renderer's set
> once that is known — it is the renderer that samples, after all — but its
> `dmabuf_formats()` covers only ARGB/XRGB/ABGR/XBGR 8888, because `vk_format` maps
> nothing else and there is no YUV sampling anywhere in that renderer. Advertising it
> would drop NV12 and every planar format from the feedback, which is what a
> hardware-decoded video client asks for.
>
> So this set is WIDER than the Vulkan draw path can import, and that is a real
> inconsistency: such a buffer is accepted at validation (which goes through GLES) and
> then fails at draw. Narrowing the advertisement is the wrong half to fix — the right
> one is either YUV support in the Vulkan renderer or a GLES route for formats it
> cannot take.

**That reasoning has been reversed, on evidence.** The premise was that keeping NV12 in
the feedback preserved something for video clients. It did not: `vk_format` never mapped
NV12, so under Vulkan those buffers were accepted at creation and then dropped at draw,
every frame — a blank window, not a slow one. Measured on this machine pre-fix: of 48
advertised fourccs only the eight the old table mapped ever drew; the rest, planar
included, were accepted and never rendered.

So the narrowing removes a **promise**, not a capability. What a video client gets
instead is an honest negotiation: it sees no NV12, picks RGB, and its frames appear.
Nothing changes on the GLES path — `advertise` only narrows to the *active* renderer's
set, and GLES really can import these.

What it does cost is the **zero-copy video path under Vulkan**: the client (or ffmpeg)
now converts to RGB itself rather than handing over the decoder's own buffer. That is a
real cost, and it is the strongest argument for prioritising the work below over
anything else in this file.

### Two routes back

1. **`VK_KHR_sampler_ycbcr_conversion`** — the textbook answer: per-format conversion
   objects, immutable samplers baked into the descriptor layout, plane-count and
   disjoint-import handling. Correct, and the most invasive.
2. **Import the planes as separate images and convert in the shader** — no ycbcr
   extension at all. NV12's planes are exactly `R8` (Y) and `R8G8` (interleaved CbCr),
   **both already mapped in Tier 0**, so the import side is mostly plumbing: two
   `VkImage`s from the same dmabuf at the per-plane offsets/strides the protocol already
   carries, then BT.601/709 conversion in the composite shader. This is what Firefox
   does with NVIDIA's exported planes, and it fits the existing table rather than
   fighting it. Note `bind_single` currently builds one image from `fds[0]`; per-plane
   images need the offsets from `plane_layouts`, which the import path already reads.

Route 2 is the cheaper experiment and would settle whether the conversion math and the
plane plumbing are right before committing to route 1's descriptor-layout surgery.

---

## Parked hardening: make the publish→advertise ordering a compile error

`advertise()` reads `compositor_importable()` from a global that
`native.wire/wire.entry` must have published first. Today that ordering is a
**convention between two statements** — publish at `entry.rs:171`,
`lifecycle::initialize` at `217`. Not a race: startup is single-threaded and
deterministic, so there is no window. The fragility is that a future refactor can
reorder them, and the failure is silent — the advertisement widens back to the
driver's whole EGL list, which is the exact bug the narrowing exists to fix.

Current state is **reporting only**: `advertise()` logs when it finds the set empty and
names the cause. Better than silence, but it is a line in a log, not enforcement.

**The fix: turn the global read into a parameter, so the value must exist to compile.**

```rust
// y5.graphic/graphic.display/display.backend/backend.rs
fn bind_display(&mut self, dh: &DisplayHandle) -> FormatSet;
fn importable(&self) -> FormatSet;            // what the ACTIVE renderer can sample
```

`NativeContract` gains an `importable: FormatSet` field, filled at construction from
the already-built renderer:

```rust
let mut contract = NativeContract {
    output: display.output.clone(),
    mode: display.mode,
    gpu_binding: renderer.gpu_binding.clone(),
    importable: match vulkan.as_ref() { … },   // ← the value must exist HERE
};
```

`register_dmabuf` then calls `backend_loader.importable()` and passes it to
`advertise(egl, importable)`. Moving the renderer back below `lifecycle::initialize`
stops being a silent widening and becomes a compile error (`vulkan` possibly
uninitialised). That is the property worth having: the ordering is enforced by the
compiler, not by the comment at `entry.rs:76`.

**Second half — remove the ambiguity that forces the soft path.** `advertise()` only
tolerates an empty set because winit never publishes. But winit *has* an importable
set (its GLES renderer's — `winit.wire/wire.entry` already calls `dmabuf_formats()` in
`bind_display`). Implement `importable()` there too and "empty" stops meaning "maybe
winit, maybe a bug"; the degraded branch can then be an `error!`/`abort!`.

**Keep the global.** `set_compositor_importable` exists for the **off-thread
producers**, which read it from other threads long after startup and cannot be handed a
parameter. That use is legitimate. Only the startup-time read inside `advertise()`
should become a parameter — after which the global has exactly one purpose instead of
two.

Scope: ~20 lines across `display.backend`, `NativeContract`, winit's contract, and
`register_dmabuf`/`advertise`.

### Related, found in the same audit (not fixed)

The **nonTB (inline) bevy and iced slots** negotiate their modifier against
`gles ∩ wgpu` (`core.slot/slot.base:40`, `monitor…/surface.rs:371`) and never consult
`compositor_importable()`. That pairing is right when GLES composites — the inline slot
builds a `GlesTexture`, so GLES importability is a hard requirement — but the inline
path also runs on **Vulkan with triple buffering off** (`registry.base:48-57` picks the
worker only when the interface is engaged). There the `GlesTexture` is skipped
(`prefers_dmabuf`) and the compositing Vulkan renderer imports a dmabuf whose modifier
nothing validated against its set. Single-GPU boxes get away with it because the three
sets coincide for `ARGB8888`; an empty intersection falls back to implicit allocation,
which `negotiate.base:101-111` itself describes as the import failure the negotiated
path exists to prevent. Fix is a three-way intersection
(`compositor_importable() ∩ gles ∩ wgpu`) at those two call sites.

The TB paths are already correct (`worker_modifiers` = `compositor_importable() ∩
wgpu`), and no producer picks an unsupported **fourcc** — everything is hardcoded
`Argb8888` except the background worker's checked ladder.

### The implicit-path landmine went off — root cause of "other windows corrupted"

Predicted in this section, then confirmed under validation layers on 2026-08-13. This is
the bug behind the long-standing report that running the format tests corrupted
*unrelated* windows (zed, nautilus).

`y5.overview/overview.draw/draw.blur` allocated its four mip buffers with
`allocate_dmabuf` — **no modifier negotiation at all**, not even the incomplete
`gles ∩ wgpu`. The log shows all four (`2560x720`, `1280x360`, `640x180`, `5120x1440`,
in blur's own half/quarter/eighth/out order):

```
gpu alloc: bevy/iced surface -> AR24 tiled (Unrecognized(216172782128496660))
    via the IMPLICIT path — no modifier list was offered, so the driver chose alone
```

`216172782128496660` = **`0x0300000000E08014`** — an NVIDIA block-linear kind *outside*
the six the device reports as importable (`0x0300000000606010`–`015`). What followed:

```
vkCreateImage(): VK_ERROR_FORMAT_NOT_SUPPORTED  drmFormatModifier (216172782128496660)
vkBindImageMemory(): memory imported from DMA_BUF but … does not report it as importable
vkCreateImageView(): B8G8R8A8_UNORM with DRM_FORMAT_MODIFIER_EXT has no supported format features
vkCmdDraw(): image view's format does not contain SAMPLED_IMAGE_FILTER_LINEAR
```

The chain did not stop at the failed create: memory was bound, a view with **no format
features** was created, and draws proceeded with that view bound as `u_tex`. A garbage
descriptor sampled in the same command buffer as every other window — which is exactly
why surfaces unrelated to the overview corrupted. GLES imports anything via EGLImage, so
the blur's own blits succeeded and nothing upstream complained.

**Fixed** by negotiating with `worker_modifiers` (compositor-importable ∩ GLES, colour
policy applied) and, critically, **refusing** when that is empty instead of falling back
to the implicit path — `allocate_dmabuf_negotiated` silently takes the implicit path when
handed an empty list, so returning `None` (caller keeps the sharp snapshot) is the only
safe degradation.

Still open: `Slot::allocate` (`bevy.core/core.slot/slot.base:40`) and
`monitor.runtime/…/surface.rs:371` intersect `gles ∩ wgpu` **without**
`compositor_importable()`, and `allocate_dmabuf_negotiated`'s empty-list-means-implicit
fallback remains a live trap for any future caller. The three-way intersection below is
still the right fix for those two sites.

### Also fixed: `vkCmdBlitImage` without `TRANSFER_SRC`

```
vkCmdBlitImage(): srcImage was created with VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT
                  but requires VK_IMAGE_USAGE_TRANSFER_SRC_BIT
```

`renderer/bind.rs` imported bound targets with `COLOR_ATTACHMENT` only, while
`capture.blit` and `mipgen` use them as a blit source. NVIDIA executed the blit anyway,
so it took validation layers to see it; a stricter driver would have produced undefined
captures. Safe to add — `vkfmt` confirms every color-attachable modifier on this device
also carries `TRANSFER_SRC`/`BLIT_SRC`, so the importable set does not narrow.

**Correction, from the output-axis work above:** this audit verified the worker ladder's
*rungs* and never its *gate*. `usable()` does check `renderable()` and a non-empty
modifier list, but the 10-bit rung is gated on `scanout_is_deep()`, which could not
become true on this hardware — so the rung had never run. The ladder is now ordered from
`scanout_fourcc()` rather than hardcoded. When auditing a ladder, check what opens it.

## The blocker

`Xid 31 … FAULT_PTE ACCESS_TYPE_VIRT_READ` from `y5_compositor`, five in one second,
then `Xid 109 CTX SWITCH TIMEOUT` and `VK_ERROR_DEVICE_LOST`; other processes'
channels die after it (`code-review-gui` copy-engine faults, nautilus shader-header
corruption) and the session freezes through a VT switch. The GPU is reading memory
that was freed or unmapped while a submitted command buffer still referenced it.

Two candidate sites, neither confirmed: textures referenced by a **barrier but never
sampled** (`pending_acquires` is not a pin, unlike `pinned_textures`), and the
**background worker**, which has its own device sharing dmabufs with the compositor and
whose fence wait reported device-lost in the same second.

Next step is instrumentation, not inspection: `vulkan-validation-layers` with
`VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION`, against a **nested** y5, with
the format tests as the trigger. Validation names the object and the submit that
destroys-while-in-use; sync validation catches a read racing a free.

### Status 2026-08-13

The `pending_acquires` candidate was **fixed** (`submit.rs` now copies the taken acquires
into `in_flight_textures`, pinning them for the frame exactly as drawn textures are).
Since then, **two runs under validation layers with no Xid, no `VK_ERROR_DEVICE_LOST`
and no freeze.** Encouraging, not proof: the crash was never reliably reproducible, and
the runs produced no `SYNC-HAZARD-*` output at all, which more likely means sync
validation never engaged than that it engaged and found nothing — `VK_LAYER_ENABLES` is
the legacy mechanism and the installed layer is 1.4.x. Re-run with a
`VK_LAYER_SETTINGS_PATH` settings file and `validate_best_practices = true` as a positive
control before believing a clean result.

The only VUID either run produced was `VUID-StandaloneSpirv-None-10684` (x7, then rate
limited): naga emits `Offset`/`ArrayStride` decorations on `Function`-storage-class
variables, which is invalid SPIR-V that NVIDIA tolerates. Vendored wgpu/naga, unrelated
to dmabuf. Filter it with `khronos_validation.message_id_filter` so it stops crowding the
log.

`NVRM: VM: invalid mmap context` bursts in the kernel log are **not** Xids — they are
CPU-side mmap rejections during address-space teardown, one per live GPU mapping, and
every burst timestamp lines up with a `pkill`. Root cause: y5 installs no SIGTERM
handler (only `SIGCHLD`, blocked for the launch reaper), so the process dies with no
unwinding, no `Drop`, no `vkDestroyDevice`. Cosmetic for now; worth an orderly shutdown
path on its own merits.

## Solved separately: the hardware-cursor garbage rectangle

Long assumed to be part of the Xid, and it was not — different layer, different bug, and
it explains why validation layers had nothing to say about it.

**Symptom.** A plane-sized rectangle of garbage at the pointer — grey scanlines,
sometimes green, sometimes recognisable fragments of other windows. Reproducible only
while the dmabuf format sweep churns allocations (`dmabuf-draw --all`, most visibly at
the tail where `R16`/`XB4H`/`AB4H` are refused with protocol error 4 and clients die);
never on an idle desktop. In one report it never recovered.

**Cause**, in `vendor/smithay/src/backend/drm/compositor/mod.rs`,
`copy_element_to_cursor_bo`. The cursor bo is allocated at `cursor_plane_size`, which is
chosen from the plane's **size hints** as the first that *holds* the element — 256x256 on
NVIDIA for a 24x24 cursor. The copy then writes only `src_height` rows and only
`src_stride` bytes per row. Rows below and columns right of the cursor image were never
written, so the bo kept whatever GBM handed back on reuse, and the cursor plane scanned
it out.

Every part of the symptom follows: needs allocation churn (quiet reuse tends to return
zeros, a busy sweep returns recent image data); content varies; no Xid and silent
validation because nothing faults — we were legitimately displaying our own
uninitialised memory; and it persists because the `render` short-circuit higher in
`try_assign_cursor_plane` skips re-rendering while element id, commit and size are
unchanged, so a bo that came back dirty stays on screen until the cursor changes.

**Fix.** Zero the destination before copying unless the element fills the bo exactly
(ARGB8888 zero is transparent, which is what the margin must be), and clamp rows and
per-row bytes to the bo — a client controls its own shm stride and height, and an
over-tall or over-padded cursor would otherwise index past the mapping and panic the
compositor. Vendored patch, marked `y5 patch` in place.

**Verify on hardware:** run `dmabuf-draw --all` and watch the pointer through the tail
where the refusals happen. (`environment.container/distributions/.src/vendor/` holds a
second copy of smithay without this patch, but it is gitignored and untracked — a staged
copy regenerated from `vendor/`, so it needs no separate edit.)

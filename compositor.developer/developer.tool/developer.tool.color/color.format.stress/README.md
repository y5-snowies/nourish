# y5 color-format-stress

Standalone Wayland clients that answer, per DRM fourcc, what the compositor
**advertises**, what it **accepts**, and what it **actually renders** — plus `vkfmt`,
which asks the **device** what it can do, with no compositor involved at all.

**Start with `./color-stress`.** It is the interactive front end: it asks the
compositor what it advertises, asks the DRM node what a client can actually allocate,
shows where the two disagree, and lets you pick a `(fourcc, modifier)` pair off either
list and render with exactly it. The other tools remain the single-purpose things it
drives. It is the
format sibling of `developer.tool.touch/touch.stress` and
`developer.tool.pen/pen.stress`, and like them it is deliberately outside
`link.all.sh` and the repo's lint rules — but it is **C**, not a crate: plain
`libwayland-client` + `libgbm`, built by the `Makefile` here.

It exists because y5 advertised formats its Vulkan renderer could not import.
`XB4H` (`Xbgr16161616f`, fp16) reached a client — vkcube picked it from the feedback —
and every frame then failed at draw with `unsupported fourcc for the vulkan path`,
which the user sees as a blank window and nothing else.

## Build & run

```bash
cd compositor.developer/developer.tool/developer.tool.color/color.format.stress
make                                              # every tool
./color-stress --socket=wayland-N                 # INTERACTIVE: query, diff, pick, render
./dmabuf-probe [wayland-N]                        # what is advertised
./dmabuf-format-test --socket=wayland-N           # accept/refuse sweep, no drawing
./dmabuf-draw --socket=wayland-N --grid --cols=4  # one window, a labelled placeholder per format
./dmabuf-draw --socket=wayland-N --all-concurrent # every format, one toplevel each
./dmabuf-draw --socket=wayland-N XR30 --size=128x128
python3 inspect.py y5-capture-*.png               # numeric read-out of a grid screenshot
./vkfmt                                           # device capability; no compositor, no surface
./dmabuf-draw --socket=wayland-N --blend AR24     # THE ALPHA TEST — see below
./dmabuf-draw --socket=wayland-N --modifier=0x300000000606014 AR24   # one exact pair
```

## `color-stress` — the interactive front end

```
  1  compositor: what is advertised          zwp_linux_dmabuf_v1 feedback (v4 tranches, or v3)
  2  device: what GBM can actually allocate  one gbm_bo_create_with_modifiers2 per pair
  3  diff: advertised but not allocatable    the interesting half
  4  pick a pair and render it               execs dmabuf-draw with --modifier=
  5  grid: every format at once              the window this directory started with
  6  blend/alpha test for one fourcc
```

Measured on this machine, nested: **644 pairs advertised, 92 a client can actually
allocate with `GBM_BO_USE_WRITE`.** That gap is not a bug on its own — `USE_WRITE` is a
CPU-write request and a GPU-rendering client may well get pairs a CPU-writing one
cannot — but it is the shape of the bug this directory exists for, so it is worth a
look when something picks a format and comes up blank.

It execs `dmabuf-draw` rather than reimplementing the renderers: one code path for what
reaches the screen. A second copy would drift, and the drift would look like a format bug.

## The alpha test (`--blend`) — and why the old one proved nothing

`sample()` hardcoded `*a = 1.0f`. Every buffer was fully opaque whatever its fourcc, so
an `AR24` placeholder and an `XR24` placeholder were byte-identical in the alpha channel and the
compositor had nothing to blend. "Alpha works" was never tested here; it was assumed.

Alpha also cannot be tested inside ONE surface: whatever the client draws IS the
surface, so a translucent pixel has nothing to blend against. That is the second half of
why the old window showed no change — the container was opaque and there was nothing
behind it.

`--blend` uses two surfaces so the compositor has to do the work:

* **parent** — an opaque `XR24` checkerboard, a format already proven byte-exact, so
  anything wrong in the result is the child's or the blend's and never the backdrop's;
* **child** — a subsurface in the format under test carrying an **alpha staircase**
  (smooth ramp on top, eight readable steps below), with **no opaque region set**, so
  the compositor cannot skip the blend.

Over the checkerboard, step *k* should read as `colour*k/7 + check*(1-k/7)`: left edge
pure checkerboard, right edge pure orange. A border of untouched checkerboard stays
visible around the child as the reference.

**An X-format must come out fully opaque here.** That is the control, not a failure —
run `--blend XR24` beside `--blend AR24` and the pair proves the compositor honours the
format's alpha semantics rather than treating all four bytes the same.

### Premultiplied, and the trap that looks like a compositor bug

Wayland surface content is **premultiplied alpha**: the stored RGB must already be
scaled by A. Every compositor blends `src + dst*(1-a)` on that assumption — y5's
pipeline is `ONE` / `ONE_MINUS_SRC_ALPHA`, and has been since `cd2c9c55`.

Hand it *straight* alpha and at `a=0` you get `colour + dst`, the full colour ADDED to
the backdrop instead of the backdrop showing through. The ramp then reads as flat
saturated colour and looks **exactly like the compositor ignoring alpha**. That is the
first version of this test's own bug, and it is worth being able to reproduce, so
`--straight` writes the wrong form deliberately:

* **premultiplied** (default) — fades to the backdrop
* **`--straight`** — blows out toward white

Telling those apart by eye is the whole point; without the pair, "no blending" is a
guess.

## CPU-written vs GPU-rendered (`--gpu`)

Everything here CPU-writes a carrier and hands it over. That is the right way to test
what the compositor does with a KNOWN byte pattern — and the wrong way to answer "does a
GPU client work", because `GBM_BO_USE_WRITE` forces **LINEAR** on this driver, so the
tiled modifiers a real client actually gets are never exercised at all.

`--gpu` allocates with `GBM_BO_USE_RENDERING`, imports the dmabuf back as an EGLImage,
binds it to an FBO and draws with GLES. The buffer is tiled, GPU-written, and never
touched by the CPU. The pattern is scissored `glClear`s rather than a shader: exact
values, no interpolation, no shader compilation to go wrong.

**It immediately found something.** Left to choose freely, `RENDERING` on this NVIDIA
device hands back the `0x0300000000e08…` modifier family — and the compositor
**soft-refuses** it (`params.failed`), because its Vulkan renderer advertises `AR24` only
in the `0x0300000000606…` family plus LINEAR. Ask for a modifier from the family the
composite can sample and the same GPU render is accepted:

```bash
./dmabuf-draw --gpu --blend --modifier=0x300000000606014 AR24   # accepted
./dmabuf-draw --gpu --blend AR24                                # gbm picks 0e08 -> refused
```

A correct client never hits this: it allocates from the modifier list the compositor
advertised. A client that calls `gbm_bo_create(..., USE_RENDERING)` with no list does,
and this is what that failure looks like from the inside.

In `--blend` the backdrop is **always** CPU-written and always plain-allocated: it is the
reference the blend is read against, so `--gpu` and `--modifier=` apply to the surface
under test, never to it.

## Choosing the modifier (`--modifier=`)

`--modifier=<hex|linear|invalid>` allocates the carrier with **exactly** that layout —
one modifier offered, so gbm either gives it or fails, and a refusal is reported rather
than silently satisfied with something else. If the driver hands back a different
modifier anyway the tool says so, and warns that the run is not testing what you asked
for. `--linear` is the special case that predates it.

`vkfmt` is the only tool here that is **safe against a live session** — it creates a
Vulkan instance, queries `vkGetPhysicalDeviceFormatProperties2` plus the DRM modifier
list, and exits. Nothing is allocated, imported, or presented. It answers exactly what
y5's gates read: `query::renderable` (`COLOR_ATTACHMENT` in `optimalTilingFeatures`),
`query::sampleable`, and `modifier::modifiers_with` (the per-modifier tiling features).
Use it to decide whether a format *could* work before reaching for the tools that can
crash the compositor. It settled two questions: the vendor modifier count is 6 and not
12, and both 10-bit channel orders are renderable on NVIDIA — so a 10-bit asymmetry is
the KMS plane's, never the GPU's.

`make regen` refreshes the wayland-scanner output; it is checked in so `make` works
without `wayland-protocols` installed.

## Why formats GBM cannot allocate are still testable

GBM will never allocate `Rgb332` or `Rg1616`. It does not need to: the compositor
never inspects pixel content, so the tools allocate a **byte-compatible CPU-writable
carrier** (`XR24` with `GBM_BO_USE_WRITE` — LINEAR on NVIDIA, where
`RENDERING|LINEAR` is refused outright) and **declare** the format under test in
`zwp_linux_buffer_params.create`. One 32bpp carrier covers every 32bpp fourcc; the
declared stride only has to be `>= width * bpp/8`.

`gbmtest.c` / `gbmtry.c` are kept because they are how that flag combination was
found, and they answer "why did allocation fail" in one run (`make gbmtry`).

## Reading `dmabuf-draw`

Each cell has a **header strip drawn in `XR24`** — the format that worked long before
any of this — naming the fourcc, bit depth and status. So a cell identifies itself
even when its body renders as garbage, or was refused and has no body at all. Below
it, the pattern in the format under test:

* **hue sweep** — a channel-order bug twists the spectrum
* **greyscale ramp** — a mis-read transfer function crushes or lifts it
* **R / G / B / white patches** — name the swapped channel outright

Statuses: `OK` accepted (`OK LIN` for the linear-light float formats) · `NOTADV`
`params.InvalidFormat`, a **protocol error** — the fourcc is not advertised, so the
compositor kills the client (expected for anything deliberately withheld) · `REFUSD`
a soft `params.failed`.

Placeholders should be *the same image* except for four deliberate cases: `R8`/`R16` are
single-channel (red ramp, black G/B patches); `RG16`/`XR15`/`BA12` band visibly;
`XR30`/`XB30` are 10-bit; and `XB4H`/`AB4H` carry the pattern **decoded to linear
light** — what a real fp16 client stores — so they match only if the compositor knows
that. A darker ramp there means it sampled linear values as if sRGB-encoded.

## Capturing the result

y5 exposes no `zwlr_screencopy` / `ext-image-copy-capture` global, so these tools
cannot screenshot themselves. Use y5's own capture (`y5.graphic/graphic.capture`, PNG
via `capture.encode/save.rs`): it reads back the **composited** frame, which is
exactly the perceived image, and writes `~/Pictures/y5-capture-<ts>.png`. Then
`inspect.py` decodes it (no PIL) and samples every cell at the geometry `--grid`
printed, so the report is pixel values rather than an impression.

`XR24` is the control: if its placeholder is exact, the readback path is trustworthy and
every other placeholder's difference is real.

## Warning

`dmabuf-format-test` **wedged a live compositor** during development: sweeping exotic
fourccs stalled it inside the NVIDIA EGL import path, on the compositor's own thread,
and it stopped answering new clients. That a client can do that is worth knowing in
itself — but point these tools at a nested compositor you can kill, never at the
session you are working in.

## Baseline measured on NVIDIA (pre-fix build, 2026-08-12)

`dmabuf-probe`: 48 fourccs × 14 modifiers advertised. `--grid` then rendered exactly
the eight the old `vk_format` mapped, and silently dropped the rest:

| result | formats |
|---|---|
| byte-exact | `XR24 AR24 XB24 AB24 XB30` |
| accepted, never drawn (backdrop showing) | `RG16 XR15 BA12 RG24 R8 R16 XB4H AB4H` |

The greys came back exactly as authored (`64/128/192`), which is also the proof that
nothing in the pipeline linearises anything today.

**Post-fix, same machine:** `XR24 AR24 XB24 AB24 XR30 XB30` byte-exact; `RG16`
(`66/132/198` — correct 5-6-5 quantisation), `XR15` (`66/132/198`, 5-bit) and `R8`
(`64/128/192` in red, black G/B patches) now render, validating three of the new
mappings; `BA12 RG24 R16 XB4H AB4H` are withheld — the first three because NVIDIA
reports no sampleable DRM modifier for `B4G4R4A4` / `B8G8R8` / `R16_UNORM` (the table
proposes, the device disposes), the last two by the fp16 colour policy.

### The black-tail artifact (was mistaken for a format bug)

Early captures showed a placeholder rendering its top rows and then pure `(0,0,0)`, and the
affected placeholder MOVED between runs — `XR30` once, `XB24` + `XR30` next, `XB30` after
that, and in one run the bottom 155 rows of the plain `XR24` **parent backdrop**. That
last one is the tell: `XR24` is otherwise byte-exact, so it was never about formats.

A freshly written carrier can be sampled before the CPU write is visible to the GPU;
the unwritten tail composites as the BO's initial zeros, and since nothing damages the
surface again, that half-drawn frame stays on screen. The tools now re-damage every
surface at 300 ms, 1 s and 2.5 s (idempotent — the content never changes). If a black
tail survives that, it IS a compositor bug and worth chasing.

## Cross-reference

Expectations here mirror
`compositor.kernel/kernel.vulkan/vulkan.format/format.table/table.base/table.rs`,
which is exhaustive over `Fourcc` (no wildcard arm). When that table changes, update
the `expect` column in `dmabuf-format-test.c` and the format list in `dmabuf-draw.c`.

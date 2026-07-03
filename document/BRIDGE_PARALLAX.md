# Bridging the parallax background: render on Vulkan (wgpu) even under GLES

This document describes the process for rendering the parallax background
**always on wgpu (Vulkan)** and compositing it through a dmabuf, so that a single
shader format (WGSL → SPIR-V) drives the background on **both** compositor
renderers — leaving the GLES renderer to merely *composite* the result instead of
running the shader itself.

Read `document/GLES_WGPU.md` first: it establishes that wgpu is already pinned to
Vulkan and that a dmabuf ↔ wgpu bridge already exists and runs even under the
GLES renderer (for the iced UI and the bevy 3D background). This design reuses
that exact bridge for the parallax background.

## Why do this

Today the background shader path forks by renderer:

- **Vulkan renderer:** WGSL/GLSL → SPIR-V via naga, full feature set.
- **GLES renderer:** native `gles/` sources only, handed raw to smithay as
  **GLSL ES 1.00** (`#version 100`, `gl_FragColor`). No `round()`, no
  dynamic-bound loops, no `out` vars — and no smithay patch to raise the version
  (reverted, by decision).

That fork is the whole reason "no single format runs on both renderers". If the
parallax is rendered on wgpu(Vulkan) and only *composited* by GLES, the fork
disappears:

- **One format** (WGSL, or anything naga lowers to SPIR-V) runs everywhere.
- No ES-1.00 ceiling; heavy shaders (space-station, snow) work under GLES too.
- **No vendored-smithay changes.**

The proof-of-concept already ships: the settings **shader preview** renders the
parallax on the iced wgpu(Vulkan) renderer via an `iced::widget::shader` widget.
The same shader, same wgpu device — this design just retargets that render from an
iced widget to a dmabuf-backed texture that the compositor composites.

## The pipeline

```
 gbm (DRM render node)                wgpu (Vulkan)                 compositor
 ─────────────────────                ─────────────                ──────────
 allocate ARGB8888  ──import once──▶  texture (RENDER_ATTACHMENT)
 LINEAR single-plane  (per buffer)        │
 dmabuf                                    │  ◀── each frame: write uniforms,
                                           │      render fullscreen parallax pass
                                           ▼
                                       signal fence  ──sync──▶  sample the SAME
                                                                dmabuf as the
                                                                background element
                                                                (GLES: EGLImage,
                                                                 Vulkan: direct)
```

### 1. Allocate the output-sized dmabuf (once per size)

Reuse `support.bevy/bevy.core/core.alloc/alloc.base::allocate_dmabuf` (or the
iced surface's `dmabuf_alloc`): a single **ARGB8888 LINEAR single-plane** buffer
from the render node. Single-plane ARGB8888 is the only layout the wgpu-HAL
dmabuf import supports. Allocate **two** for double-buffering (see §5).

### 2. Import into the shared wgpu(Vulkan) context (once per buffer)

Use the shared `WgpuVulkanContext` and `import_dmabuf_to_wgpu` (already used by
iced and bevy). The import produces a `Bgra8UnormSrgb` texture with
`RENDER_ATTACHMENT` usage. **This happens once per dmabuf**, not per frame —
re-import only when the output is resized (which reallocates the buffer). The
texture *handle* is stable; only its *contents* change per frame.

### 3. Compile the parallax shader to a wgpu pipeline (once per shader)

Reuse the existing runtime compile path
(`background.two/two.shader/shader.spirv::build_wgsl` and the preview's
`glsl_to_preview_wgsl` for GLSL sources). The parallax uniforms (`u_time`,
`u_pan`, `u_zoom`, `u_flow_offset`, `u_resolution`, `u_lock_amount`, and the two
`@prop` `param` vec4 slots) become a uniform/std140 buffer or push constants —
same data the existing Vulkan `FullscreenPass` already carries.

### 4. Render + sync (every frame)

Per frame, on the wgpu side:

1. **Write uniforms** (time advances, pan/zoom/flow change — the background is
   animated, so contents must be redrawn every frame; a static shader could skip
   this).
2. **Render** the fullscreen parallax pass into the dmabuf-backed texture.
3. **Submit** and **signal a fence** the compositor can wait on.

Then on the compositor side, **synchronize** (wait for the fence / implicit sync)
before sampling. This ordering is mandatory every frame — it is the read-after-
write hazard between the wgpu producer and the compositor consumer.

> This is the answer to "why per frame and not once": **import** is once-per-
> buffer; **render + composite + sync** are per-frame because the animation
> changes the texture *contents*, not the texture *handle*.

### 5. Composite as the background element

Feed the (now-rendered, synced) dmabuf to the compositor as the bottom-most
background element. Under Vulkan the compositor samples it directly; under GLES it
imports the dmabuf as an `EGLImage` and samples that. Either way the GLES path no
longer runs any pixel shader — it just textures a quad.

**Double-buffer** (ping-pong the two dmabufs from §1): render frame N+1 into
buffer B while the compositor is still sampling buffer A from frame N. This avoids
a stall and the read/write race on a single buffer.

## What gets removed / simplified

- The GLES-native background pixel-shader path
  (`draw.program::compile_source` + smithay `compile_custom_pixel_shader`) is no
  longer on the critical path for user shaders — GLES becomes a pure compositor of
  the bridged texture. (Keep it as an ultimate fallback if wgpu init fails.)
- The `gles/` bundle format and the whole `shaderFormat × renderer` matrix
  collapse toward "author WGSL (or desktop GLSL), runs everywhere".

## Risks and considerations

- **DRM modifier divergence (tiling corruption).** The bridge negotiates no DRM
  modifiers — it allocates with the implicit driver modifier and imports
  single-plane. On some GPUs (observed on AMD) the compositor's sampling/scanout
  path and the bridge can disagree on tiling and corrupt the image. **LINEAR
  allocation is the current mitigation**; a real fix negotiates an explicit,
  shared modifier on both ends. This is the single biggest correctness risk and
  must be validated per-GPU.
- **Write-every-frame quirk.** In the bevy bridge, a bridged texture stays on the
  blank placeholder unless the material is actually written through each frame
  (a get-mut alone marks nothing). The parallax equivalent: ensure the pass is
  genuinely re-encoded/submitted each frame, not skipped when "nothing changed" —
  an animated background always changes.
- **Synchronization cost.** The per-frame fence wait couples the compositor's
  frame to wgpu's submission. Budget for it; double-buffering hides most of it.
- **Vulkan becomes mandatory for the background too.** Already effectively true
  (see `GLES_WGPU.md`: the UI needs Vulkan-via-wgpu regardless), so this adds no
  new hardware requirement — but the built-in GLES pixel-shader fallback should
  remain for the (already unsupported) no-Vulkan case.
- **Extra memory + a render target per output.** One (doubled: two) output-sized
  ARGB8888 buffer per output, plus the wgpu pipeline. Modest, but not free on
  many-output setups.

## Reused building blocks (nothing here is new infrastructure)

| Need | Existing crate / symbol |
|---|---|
| gbm dmabuf allocation | `support.bevy/bevy.core/core.alloc/alloc.base::allocate_dmabuf`; `.../surface.base/src/dmabuf_alloc.rs` |
| shared Vulkan wgpu device | `support.bevy/.../core.context::WgpuVulkanContext`; `.../surface.base/src/wgpu_context.rs::create_wgpu_vulkan_context` |
| dmabuf → wgpu texture import | `import_dmabuf_to_wgpu` (bevy `core.import` and iced `wgpu_import.rs`) |
| shader → SPIR-V / preview WGSL | `background.two/two.shader/shader.spirv::build_wgsl`, `glsl_to_preview_wgsl` |
| fullscreen parallax pass + uniforms | `two.draw/draw.vulkan` `FullscreenPass`; `draw.motion::uniforms` |
| proof it renders on wgpu | the settings shader **preview** (`iced::widget::shader`) |

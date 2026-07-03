# GLES renderer, Vulkan reality: where wgpu (Vulkan) runs even under GLES

y5 has two compositor render paths — **Vulkan** (udev/native, the primary path)
and **GLES** (winit / fallback). But "the GLES renderer" is a misnomer for the
*whole* frame: even when the compositor is compositing with GLES, several large
subsystems still run on **Vulkan via wgpu**. This document explains what runs
where, and why the split is not configurable.

## TL;DR

- **wgpu is always Vulkan here.** It is not auto-selected; two independent call
  sites hard-code `wgpu::Backends::VULKAN`.
- Under the **GLES** compositor renderer, Vulkan is still live for: the **iced
  UI** (settings, overview, the shader **preview**), the **bevy 3D background**,
  and all **dmabuf bridging** between those and the compositor.
- GLES only does the smithay **compositing** itself: window/damage/presentation
  and the built-in parallax **pixel-shader background**.
- Therefore a **Vulkan-capable GPU is required regardless** of renderer choice.
  If Vulkan were absent, the iced UI would not come up even under GLES.

## Why wgpu is pinned to Vulkan

wgpu's default backend selection on Linux would already *prefer* Vulkan, but here
it is **forced**, not merely preferred:

- Iced UI wgpu instance —
  `compositor.monitor/monitor.runtime/runtime.surface/surface.base/src/wgpu_context.rs`
  (`create_wgpu_vulkan_context`): `backends: wgpu::Backends::VULKAN`, logs
  *"Created wgpu::Instance (Vulkan backend)"*.
- Bevy background wgpu instance —
  `compositor.support/support.bevy/bevy.core/core.context/context.base/lib.rs`:
  `backends: wgpu::Backends::VULKAN`.

The reason it *must* be Vulkan: the dmabuf ↔ wgpu texture bridge uses the wgpu
**Vulkan HAL escape hatch** —

```rust
let hal_device_guard = ctx.device.as_hal::<wgpu::hal::api::Vulkan>();
hal_device.texture_from_dmabuf_fd(fd, &hal_desc, modifier, stride, offset)?;
// then
ctx.device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(hal_texture, &desc)
```

(see `.../surface.base/src/wgpu_import.rs` and
`support.bevy/bevy.core/core.import/import.base/lib.rs`). This API only exists on
the Vulkan HAL backend, and it depends on two Vulkan device features that the
context init hard-requires:

```rust
Features::VULKAN_EXTERNAL_MEMORY_FD | Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF
```

A GL wgpu backend has no equivalent, so importing a gbm dmabuf into wgpu (and
sharing it with the compositor) is impossible off Vulkan. Hence the hard pin.

## The dmabuf bridge (how wgpu content reaches the screen)

Both the iced UI and the bevy background share GPU memory with the compositor
through the same pattern:

1. **Allocate** an `ARGB8888` **LINEAR, single-plane** dmabuf from a DRM render
   node via gbm (`support.bevy/bevy.core/core.alloc/alloc.base/lib.rs`,
   `.../surface.base/src/dmabuf_alloc.rs`). Single-plane ARGB8888 is what the
   single-plane wgpu-HAL import supports.
2. **Import** that dmabuf into the shared wgpu(Vulkan) device as a
   `Bgra8UnormSrgb` texture with `RENDER_ATTACHMENT` usage (`import_dmabuf_to_wgpu`).
   gbm's `Argb8888` maps to BGRA in API endianness; sRGB keeps text/content
   correct.
3. **Render** into that texture with wgpu (iced draws its UI; bevy draws the 3D
   scene).
4. **Composite** — the compositor samples the *same* dmabuf (GLES imports it as
   an `EGLImage`; Vulkan samples it directly) and blends it into the final frame.

The dmabuf handle is imported **once** per buffer (reallocated only on output
resize). What happens every frame is the **wgpu render** + the **composite** +
**synchronization** (the compositor must wait for wgpu's submission to finish
before it samples). See `document/BRIDGE_PARALLAX.md` for the per-frame vs
per-import breakdown applied to the parallax background.

## What this means in practice

| Subsystem | Vulkan renderer | GLES renderer |
|---|---|---|
| Window compositing / damage / presentation | Vulkan | **GLES** |
| Built-in parallax pixel-shader background | Vulkan | **GLES** (raw ES 1.00) |
| Runtime user shader background | Vulkan (WGSL/GLSL → SPIR-V) | **GLES** native `gles/` only |
| iced UI (settings, overview, shader preview) | Vulkan (wgpu) | **Vulkan (wgpu)** |
| bevy 3D background | Vulkan (wgpu) | **Vulkan (wgpu)** |
| dmabuf ↔ wgpu bridging | Vulkan HAL | **Vulkan HAL** |

The consequence worth internalizing: the settings **shader preview always uses
Vulkan**, regardless of the compositor renderer, because it is an iced
`shader` widget on the iced wgpu (Vulkan) renderer — completely independent of
the smithay GLES/Vulkan path. That is why a shader can preview correctly and yet
fall back to the built-in when the *background* is drawn on the GLES renderer
(the GLES background path only consumes native `gles/` ES-1.00 sources).

## Known bridge caveats

- **DRM modifier divergence (tiling corruption).** The internal gbm → wgpu
  bridge negotiates no DRM modifiers — it allocates with the implicit driver
  modifier and imports single-plane. On some GPUs (observed on AMD) the
  compositor's scanout path and the bridge disagree on tiling, producing
  corruption. LINEAR allocation is the current mitigation.
- **Single-plane only.** The wgpu-HAL dmabuf import does not support multi-plane
  today; ARGB8888 LINEAR stays single-plane, which is why that format is
  canonical throughout the bridge.

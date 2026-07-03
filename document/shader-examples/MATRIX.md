# Shader format × renderer compatibility matrix

How the runtime loader handles each **bundle format** on each **renderer**. The
loader tries the active renderer's formats in a fixed order and uses the first
that compiles; a renderer only attempts its *native* formats, so a bundle runs
on a renderer only if it ships a format that renderer consumes.

- **Vulkan** (the primary renderer) tries: `vulkan/` → `wgsl/` → `glsl/`, all
  compiled to SPIR-V via naga.
- **GLES** (winit / fallback) tries: `gles/`, handed raw to smithay's
  `compile_custom_pixel_shader` as **GLSL ES 1.00** (`#version 100`,
  `gl_FragColor`). No vendored-smithay patch, so ES-3-only builtins
  (`round()`, dynamic-bound loops, `out` vars) are unavailable on GLES.

| Bundle format | Vulkan renderer | GLES renderer |
|---|---|---|
| `vulkan/shader.wgsl` | ✅ WGSL → SPIR-V | ⛔ not attempted → built-in fallback |
| `wgsl/shader.wgsl` | ✅ WGSL → SPIR-V | ⛔ not attempted → built-in fallback |
| `glsl/shader.frag` (desktop 450) | ✅ naga glsl-in → SPIR-V | ⛔ not attempted → built-in fallback |
| `gles/shader.frag` (ES 1.00) | ⛔ not attempted → built-in fallback | ✅ raw GLSL ES 1.00 (no ES-3 builtins) |

**To run on BOTH renderers**, a bundle must ship a Vulkan format *and* a `gles/`
format (see `mtx-both` / `dual`). The ⛔ cells are "not attempted", not
"impossible": filling them would mean cross-compiling (e.g. GLES trying
`wgsl/`/`glsl/` via naga → ES-3 GLSL; Vulkan trying `gles/` via an ES→desktop
shim) — not currently wired.

**Preview** (settings, wgpu): `wgsl/`/`vulkan/` render live; `glsl/` renders via
glsl→WGSL cross-compile; `gles/`-only bundles show the built-in as a stand-in.

## Test bundles (installed under `~/.local/share/y5/background/shader/`)

Each exercises the feature that *proves* its cell. Select in **Settings →
Current world → Background shader**, then switch renderers (`run-host.sh udev`
= Vulkan, `run-host.sh winit` = GLES).

| Bundle | Cell it tests | Proof feature |
|---|---|---|
| `mtx-wgsl` | `wgsl/` on Vulkan | native WGSL (`var<immediate>` push, `array<vec4>`) |
| `mtx-vulkan` | `vulkan/` on Vulkan | explicit Vulkan WGSL folder |
| `mtx-glsl` | `glsl/` on Vulkan | `round()` — a builtin absent from ES 1.00 |
| `mtx-gles` | `gles/` on GLES | plain ES 1.00 (`gl_FragColor`, no ES-3 builtins) |
| `mtx-both` | both renderers | WGSL on Vulkan + ES 1.00 GLSL on GLES, identical look |

Expected results:
- On **Vulkan**: `mtx-wgsl`, `mtx-vulkan`, `mtx-glsl`, `mtx-both` render;
  `mtx-gles` falls back to the built-in (no Vulkan format).
- On **GLES**: `mtx-gles`, `mtx-both` render only if their `gles/` source stays
  within ES 1.00; `mtx-wgsl`/`mtx-vulkan`/`mtx-glsl` fall back to the built-in
  (no `gles/` format). A `gles/` source using ES-3 features fails to compile and
  also falls back.
- Each exposes a `steps`/`speed` slider — edit it and watch the background +
  preview update.

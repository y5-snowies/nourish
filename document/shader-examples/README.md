# Background shader examples

Example bundles for the runtime parallax-background shader pipeline. Each is a
folder under `<name>/` containing one or more **format subfolders**; the loader
auto-detects the format and, for the active renderer, tries the available
formats in that renderer's preferred order, falling back to the built-in
parallax if none compile.

| Example | Folder(s) | Vulkan | GLES | Exercises |
|---|---|---|---|---|
| `aurora` | `wgsl/` | WGSL → SPIR-V ✓ | falls back to built-in | single-source WGSL |
| `plasma` | `glsl/` | GLSL(450) → SPIR-V ✓ | falls back to built-in | single-source desktop GLSL + generated vertex |
| `dual` | `vulkan/` + `gles/` | WGSL → SPIR-V ✓ | raw ES-1.00 ✓ | explicit per-backend authoring |
| `ripple` | `gles/` | falls back to built-in | raw ES-1.00 ✓ | native-GLES-only + per-renderer fallback |
| `broken-wgsl` | `wgsl/` | compile error → built-in | built-in | WGSL compile-error fallback |
| `broken-glsl` | `glsl/` | compile error → built-in | built-in | GLSL compile-error fallback |

Notes:
- The Vulkan backend is the primary path (`compositor_prefers_dmabuf`). `wgsl/`
  and `glsl/` are Vulkan-only here: emitting GLES from naga needs an ES-3.00
  smithay path we deliberately don't add, so on GLES they fall back to built-in.
- Vulkan shaders use the standard 48-byte engine `Push`
  (`res_zoom_time` / `pan_flow` / `lock_alpha`); GLES shaders declare the engine
  uniforms they use (`u_time`, `u_resolution`, …) and write `gl_FragColor`.
- `// @prop name kind k=v …` lines declare tunable variables. Each prop maps to
  a fixed param slot (prop #i → `u_param0/1` on GLES, the push `params` block on
  Vulkan); the shader must actually read its slot to react (the examples here
  do). The settings **Current World** tab renders a slider/toggle per prop,
  edits drive the live background and persist per-world, and a live wgpu preview
  (drag to pan, scroll to zoom) sits at the top.

## Install

Copy into the user data dir the loader scans:

```sh
mkdir -p ~/.local/share/y5/background/shader
cp -r document/shader-examples/* ~/.local/share/y5/background/shader/
```

Then pick one per world from **Settings → Current world → Background shader**, or
set the new-world default in `~/.config/y5.compositor/preferences.json`:

```json
{ "background_shader": "aurora" }
```

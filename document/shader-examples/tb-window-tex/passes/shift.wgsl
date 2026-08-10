// TB STRESS 7/10 — "tb-window-tex", pass 2/2 (AFTER-CONTENT, bindless textures).
//
// A THREE-STATE DIAGNOSTIC. Earlier versions of this pass blended by the sampled
// alpha, so an unbound texture array produced alpha 0 and the pass was invisible
// — indistinguishable from "the bundle never loaded". Nothing here is gated on
// the data being tested:
//
//   1. MAGENTA BEACON, top-left corner block — drawn unconditionally. If you see
//      it, this after-content pass RAN.
//   2. GREEN COUNT BLOCKS along the top edge — one per window in the rects UBO.
//      Beacon but no blocks => the window-set is empty (no client windows, or
//      collection broke).
//   3. THUMBNAIL WALL along the bottom — each window's own texture, sampled
//      through the bindless array, drawn OPAQUE (alpha shown separately as a
//      corner swatch, never used to blend). Count blocks but no thumbnails =>
//      the texture array is not bound while the rects are.
//
// If you see NONE of the three, the bundle did not load at all: `load_multipass`
// gates `window-textures` on VulkanDevice::descriptor_indexing and returns None
// where it is absent, falling the whole bundle back to the single-pass path (you
// get the STOCK background). That case logs a warn! — check the log to confirm.
//
// Why this is the hardest case for triple buffering: these are CLIENT dmabufs on
// the compositor's device. Running this pass on the worker means importing them
// into the worker's device directly and honouring each client's explicit-sync
// acquire fence — strictly more machinery than any other engine interface.
//
// textureSampleLevel throughout: per-window indexing is non-uniform control flow.

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    var col = textureSampleLevel(scene, samp, uv, 0.0).rgb;
    let n = min(windows.count, 256u);

    // (1) BEACON — unconditional proof the pass ran.
    if (uv.x < 0.04 && uv.y < 0.04) {
        return vec4<f32>(1.0, 0.0, 0.9, 1.0);
    }

    // (2) COUNT BLOCKS — one green cell per window, top edge.
    if (uv.y > 0.045 && uv.y < 0.075) {
        let slot = u32(floor(uv.x * 32.0));
        if (slot < n && fract(uv.x * 32.0) < 0.8) {
            return vec4<f32>(0.1, 1.0, 0.35, 1.0);
        }
    }

    // (3) THUMBNAIL WALL — bottom strip, one cell per window, drawn OPAQUE.
    // NON-UNIFORM INDEXING HAZARD: `win_tex` is a `binding_array` and naga emits
    // no `NonUniform` decoration, so indexing it with a PER-PIXEL value is
    // undefined — the driver resolves one lane's index for the whole wave and
    // every pixel samples the same window ("each window shows the OTHER window").
    // Sample INSIDE the loop, indexed by the wave-uniform loop counter.
    if (uv.y > 0.80 && n > 0u) {
        let cells = min(n, 6u);
        let cw = 1.0 / f32(cells);
        let local = vec2<f32>(fract(uv.x / cw), (uv.y - 0.80) / 0.20);
        // Cell border so empty cells are still visible.
        if (local.x < 0.02 || local.x > 0.98 || local.y < 0.02 || local.y > 0.98) {
            return vec4<f32>(0.95, 0.85, 0.1, 1.0);
        }
        var px = vec4<f32>(0.0);
        for (var i = 0u; i < cells; i = i + 1u) {
            if (u32(floor(uv.x / cw)) == i) {
                let s = windows.srcs[i];
                px = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
            }
        }
        // Alpha swatch in the cell's top-left, NEVER used to blend.
        if (local.x < 0.12 && local.y < 0.12) {
            return vec4<f32>(vec3<f32>(px.a), 1.0);
        }
        return vec4<f32>(px.rgb, 1.0);
    }
    return vec4<f32>(col, 1.0);
}

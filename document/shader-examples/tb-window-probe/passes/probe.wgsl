// tb-window-probe pass 2/2 — GROUND TRUTH for the window set.
//
// Answers three questions at a glance, so we stop guessing which array is wrong:
//
//   1. WHICH INDEX covers which window. Each window's rect is split down the
//      middle; the LEFT half is a flat colour keyed to its index i:
//        i=0 RED   i=1 GREEN   i=2 BLUE   i=3 YELLOW   i=4 MAGENTA   i≥5 GREY
//   2. WHICH TEXTURE that index holds. The RIGHT half shows `win_tex[i]`,
//      sampled through `srcs[i]`, stretched across the half.
//      => If the right half shows a DIFFERENT window's content than the window
//         the rect sits on, rects[] and the texture array disagree.
//   3. WHETHER `srcs` IS SANE. The bottom strip draws one bar per window, length
//      = srcs[i].z (crop width, normally ~1.0), in that index's colour.
//      => Bars near zero mean the crop is degenerate, which collapses every
//         sample to one texel — that alone makes windows look black/empty.
//
// Top-left: one white block per window in `windows.count`.
//
// All `win_tex` reads use the loop counter (wave-uniform). Indexing a
// binding_array with a per-pixel value is undefined; see SHADER_PIPELINE.md §3.

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

fn idx_colour(i: u32) -> vec3<f32> {
    if (i == 0u) { return vec3<f32>(1.0, 0.15, 0.15); }
    if (i == 1u) { return vec3<f32>(0.15, 1.0, 0.25); }
    if (i == 2u) { return vec3<f32>(0.25, 0.45, 1.0); }
    if (i == 3u) { return vec3<f32>(1.0, 0.9, 0.2); }
    if (i == 4u) { return vec3<f32>(1.0, 0.3, 1.0); }
    return vec3<f32>(0.6, 0.6, 0.6);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    var col = textureSampleLevel(scene, samp, uv, 0.0).rgb;
    let n = min(windows.count, 256u);

    // (1) count readout — one white block per window, top-left.
    if (uv.y < 0.03) {
        let slot = u32(floor(uv.x * 40.0));
        if (slot < n && fract(uv.x * 40.0) < 0.75) {
            return vec4<f32>(1.0, 1.0, 1.0, 1.0);
        }
    }

    // (3) srcs sanity bars — bottom strip, one row per window.
    if (uv.y > 0.90 && n > 0u) {
        let row = u32(floor((uv.y - 0.90) / (0.10 / f32(n))));
        for (var i = 0u; i < n; i = i + 1u) {
            if (row == i && uv.x < windows.srcs[i].z) {
                return vec4<f32>(idx_colour(i), 1.0);
            }
        }
    }

    // (1)+(2) per-window split: index colour | that index's texture.
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let lo = r.xy;
        let hi = r.xy + r.zw;
        if (uv.x >= lo.x && uv.x <= hi.x && uv.y >= lo.y && uv.y <= hi.y) {
            let local = (uv - lo) / max(r.zw, vec2<f32>(0.0001));
            if (local.x < 0.5) {
                col = idx_colour(i);
            } else {
                let s = windows.srcs[i];
                let stretched = vec2<f32>((local.x - 0.5) * 2.0, local.y);
                col = textureSampleLevel(win_tex[i], samp, s.xy + stretched * s.zw, 0.0).rgb;
            }
        }
    }
    return vec4<f32>(col, 1.0);
}

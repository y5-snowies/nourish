// STAGE 5b VALIDATOR — per-window textures on the WORKER.
//
// Until this bundle existed, nothing exercised stage 5b at all. glass, mirror and
// xray all look like they would, but every one of them has an after-content pass,
// which makes them `Offload::BeforeBand` — and `worker_can_render` only offloads
// `Whole`. So they ran inline before 5b and inline after it, and "they still work"
// said nothing about the cross-device path.
//
// This bundle is the first that is BOTH `Whole` and needs window textures: one
// before-content pass, no post-composite inputs, no `windows: pipeline`. With
// triple buffering on it therefore runs on the worker, and its window textures
// have to have crossed a device boundary to be sampled at all — a client dmabuf
// imported directly, or a SHM surface reconstructed from an OPAQUE_FD share of
// the image the compositor uploaded into.
//
// It draws a THUMBNAIL STRIP along the bottom: one cell per window, each showing
// that window's own texture, with an index-coloured border
// (0 red, 1 green, 2 blue, 3 yellow, 4 magenta).
//
// | What you see | Meaning |
// |---|---|
// | thumbnails show real window content | 5b works — textures crossed to the worker |
// | cells present but black/empty | the array is bound but the imports produced nothing |
// | no strip at all | `windows.count` is 0, or the bundle fell back to single-pass |
// | thumbnails show the WRONG window | rects and textures disagree — the index-alignment bug |
//
// Because it is a background pass, real windows composite OVER it: look at an
// uncovered part of the desktop. Drag a window over the strip and its own
// thumbnail keeps updating underneath.
//
// Confirm routing in the log: `background worker: pane running graph offthread`.
// Compare against `tb-stage-5b-inline`, which pins the same graph to the
// compositor — the strips should be identical.
//
// win_tex is indexed by the loop counter only (SHADER_PIPELINE.md §3).

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;

// `_pad: vec3<u32>` is 16-byte aligned: the arrays start at offset 32.
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
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;

    // Moving backdrop, so a frozen worker is obvious independently of the strip.
    let sweep = 0.5 + 0.5 * sin((uv.x + uv.y) * 7.0 - t * 0.5);
    var col = mix(vec3<f32>(0.04, 0.05, 0.10), vec3<f32>(0.10, 0.07, 0.18), sweep);

    let n = min(windows.count, 256u);
    if (uv.y > 0.72 && n > 0u) {
        let cells = min(n, 5u);
        let cw = 1.0 / f32(cells);
        let local = vec2<f32>(fract(uv.x / cw), (uv.y - 0.72) / 0.28);
        let cell = u32(floor(uv.x / cw));
        // Index-coloured border, so which slot a thumbnail came from is legible.
        if (local.x < 0.03 || local.x > 0.97 || local.y < 0.03 || local.y > 0.97) {
            return vec4<f32>(idx_colour(min(cell, 4u)), 1.0);
        }
        // Sample with the LOOP COUNTER: a binding_array index must be uniform.
        var px = vec4<f32>(0.0);
        for (var i = 0u; i < cells; i = i + 1u) {
            if (i == cell) {
                let s = windows.srcs[i];
                px = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
            }
        }
        return vec4<f32>(px.rgb, 1.0);
    }
    return vec4<f32>(col, 1.0);
}

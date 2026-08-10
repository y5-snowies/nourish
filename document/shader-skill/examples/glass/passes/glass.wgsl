// glass pass 2/2 — "glass" (AFTER-CONTENT). The req-6 frosted-glass effect,
// authored ENTIRELY as a shader over the pipeline-agnostic engine interfaces —
// no native pipeline:
//   • window-rects   (@group(1) @binding(0)) — each CLIENT WINDOW's on-screen
//     rectangle (client surfaces only; iced-world panels/placeholders are NOT
//     windows and never appear here)
//   • window-textures (@group(1) @binding(1)) — the bindless per-window texture
//     array, index-aligned with the rects; the window's own PREMULTIPLIED alpha
//     is the mask (opaque text stays crisp, translucent fills frost)
//   • history         (previous frame's world content) — the blurred backdrop
//   • content         — the background outside every window
//
// Per pixel: find the front-most window covering it, sample that window's texture
// for its rgba, and composite it (premultiplied over) onto a disk-blurred sample
// of `history`. Outside all windows we keep `content` (the wallpaper). Needs the
// device's descriptor-indexing feature; the producer falls back otherwise.
//
// textureSampleLevel (explicit LOD) is used throughout: the per-window branch is
// non-uniform control flow, where implicit-LOD textureSample is illegal.
//
// @prop tint float default=0.85 min=0.0 max=1.5   step=0.01 label="Backdrop brightness" group="Glass"
// @prop blur float default=28.0 min=0.0 max=96.0  step=1.0  label="Frost radius px"      group="Glass"

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,  // @prop: 0.x = tint, 0.y = blur px
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var content_tex: texture_2d<f32>;
@group(0) @binding(2) var history_tex: texture_2d<f32>;

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,  // xy = screen origin (uv), zw = screen size (uv)
    srcs: array<vec4<f32>, 256>,   // xy = texture-crop origin, zw = size (texture uv)
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;

// Disk blur of the history backdrop (24 taps, 2 rings) — the frost.
fn frost(uv: vec2<f32>, res: vec2<f32>, radius_px: f32) -> vec3<f32> {
    let r = radius_px / max(res, vec2<f32>(1.0));
    var acc = textureSampleLevel(history_tex, samp, uv, 0.0).rgb;
    var w = 1.0;
    for (var i = 0; i < 12; i = i + 1) {
        let a = f32(i) * 0.5235988; // 30°
        let dir = vec2<f32>(cos(a), sin(a));
        acc = acc + textureSampleLevel(history_tex, samp, uv + dir * r, 0.0).rgb;
        acc = acc + textureSampleLevel(history_tex, samp, uv + dir * r * 0.5, 0.0).rgb;
        w = w + 2.0;
    }
    return acc / w;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let uv = frag.xy / max(res, vec2<f32>(1.0));
    let tint = pc.params[0].x;    // @prop tint
    let blur_px = pc.params[0].y; // @prop blur

    // Front-most window covering this pixel = the LAST rect that contains it
    // (windows are uploaded back-to-front, so a higher index draws on top).
    // NON-UNIFORM INDEXING HAZARD — read before editing.
    // `win_tex` is a `binding_array`, and naga emits no `NonUniform` decoration.
    // Indexing it with a PER-PIXEL value (the winning window) is undefined: the
    // driver resolves one lane's index for the whole wave, so every pixel in that
    // wave samples the same window — which reads on screen as "each window shows
    // the OTHER window". The loop counter is uniform across the wave, so the
    // texture must be sampled INSIDE the loop, indexed by `i`, and the result kept.
    var found: i32 = -1;
    var win = vec4<f32>(0.0);
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let lo = r.xy;
        let hi = r.xy + r.zw;
        if (uv.x >= lo.x && uv.y >= lo.y && uv.x <= hi.x && uv.y <= hi.y) {
            found = i32(i);
            // Map the in-window position through the surface's texture crop —
            // buffers are often larger than the drawn region (HiDPI / fractional
            // scale / viewporter), so sampling [0,1] would stretch or show padding.
            let win_uv = (uv - lo) / max(r.zw, vec2<f32>(1e-4));
            let src = windows.srcs[i];
            win = textureSampleLevel(win_tex[i], samp, src.xy + win_uv * src.zw, 0.0);
        }
    }

    if (found < 0) {
        // Outside every window: the wallpaper as composited.
        return vec4<f32>(textureSampleLevel(content_tex, samp, uv, 0.0).rgb, 1.0);
    }

    // Alpha-aware glass. The window texture is premultiplied (Wayland), so
    // out = win.rgb + backdrop*(1 - win.a) is a premultiplied "over": opaque
    // pixels show the window untouched, translucent fills reveal the frost.
    let backdrop = frost(uv, res, blur_px) * tint;
    let outc = win.rgb + backdrop * (1.0 - win.a);
    return vec4<f32>(outc, 1.0);
}

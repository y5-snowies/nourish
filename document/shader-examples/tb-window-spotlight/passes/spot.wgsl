// TB STRESS 12 — "tb-window-spotlight", pass 2/2 (AFTER-CONTENT, rects ONLY).
//
// The LOUD rects test — and the one that can never silently fall back, because
// `window-rects` is just a UBO of numbers: no bindless array, no descriptor
// indexing, so `load_multipass` has nothing to gate on. If this bundle does not
// look dramatic, the problem is the window-set itself, not device capability.
//
// Everything OUTSIDE every window is crushed to near-black and desaturated.
// Everything INSIDE is boosted, with animated diagonal stripes and a thick
// border. Drag a window and the spotlight must track it exactly — any lag is the
// frame gap between the band that produced the rects and the band that drew them,
// which is precisely what a paced worker would introduce.
//
// With NO windows open the whole screen goes dark and a red corner marker shows
// — so "no windows" is visually distinct from "shader not running".
//
// @prop dim    float default=0.12 min=0.0 max=1.0  step=0.01 label="Outside level" group="Spotlight"
// @prop boost  float default=1.15 min=0.5 max=2.5  step=0.01 label="Inside boost"  group="Spotlight"
// @prop stripe float default=1.0  min=0.0 max=3.0  step=0.01 label="Stripes"       group="Spotlight"
// @prop speed  float default=3.0  min=0.0 max=12.0 step=0.1  label="Stripe speed"  group="Spotlight"

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
};
@group(1) @binding(0) var<uniform> windows: Windows;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let src = textureSample(scene, samp, uv).rgb;
    let n = min(windows.count, 256u);

    // No windows at all -> unmistakable red corner marker.
    if (n == 0u && uv.x < 0.05 && uv.y > 0.95) {
        return vec4<f32>(1.0, 0.1, 0.1, 1.0);
    }

    var inside = 0.0;
    var edge = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let lo = r.xy;
        let hi = r.xy + r.zw;
        if (uv.x >= lo.x && uv.x <= hi.x && uv.y >= lo.y && uv.y <= hi.y) {
            inside = 1.0;
            let local = (uv - lo) / max(r.zw, vec2<f32>(0.0001));
            let e = min(min(local.x, 1.0 - local.x), min(local.y, 1.0 - local.y));
            if (e * min(r.zw.x * res.x, r.zw.y * res.y) < 4.0) {
                edge = 1.0;
            }
        }
    }

    let lum = dot(src, vec3<f32>(0.299, 0.587, 0.114));
    let dark = vec3<f32>(lum * pc.params[0].x);          // outside: crushed
    let stripe = 0.5 + 0.5 * sin((uv.x + uv.y) * 140.0 - t * pc.params[0].w);
    // inside: boosted
    let lit = src * pc.params[0].y
        + vec3<f32>(0.10, 0.06, 0.0) * stripe * pc.params[0].z;
    var col = mix(dark, lit, inside);
    col = mix(col, vec3<f32>(1.0, 0.85, 0.1), edge);
    return vec4<f32>(col, 1.0);
}

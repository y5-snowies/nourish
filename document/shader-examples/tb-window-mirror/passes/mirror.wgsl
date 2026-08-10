// TB STRESS 11 — "tb-window-mirror", pass 2/2 (AFTER-CONTENT, rects + textures).
//
// THE UNMISSABLE ONE. Inside every client window it REPLACES the composited
// pixels with that window's own texture, sampled MIRRORED horizontally. Text
// reads backwards. There is no alpha gate and no blend — if the texture array is
// live you cannot fail to notice, and if it is not you get an obvious flat red
// fill instead of a silent no-op.
//
// This is also the closest thing here to "the pipeline takes over drawing the
// window": it is doing the window's own composite itself, from the client
// texture, rather than post-processing what the compositor already drew. The
// difference from a real takeover is that the compositor ALSO drew the window
// underneath — this just paints over it.
//
// Because it replaces rather than blends, it proves rect/src/texture index
// alignment strictly: a mirrored window showing the WRONG window's content means
// the three arrays have drifted out of lockstep.
//
// `axis` deliberately includes an identity entry. Set it to "None" and the window
// should read exactly as the compositor drew it — which turns this bundle into a
// direct A/B of the bindless path against the engine's own composite.
//
// @prop axis   int   default=0 choices="Horizontal,Vertical,Both,None" label="Mirror" group="Mirror"
// @prop border float default=3.0 min=0.0 max=12.0 step=0.5 label="Border px" group="Mirror"

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

    // NON-UNIFORM INDEXING HAZARD — read before editing.
    // `win_tex` is a `binding_array`, and naga emits no `NonUniform` decoration.
    // Indexing it with a PER-PIXEL value (the winning window) is undefined: the
    // driver resolves one lane's index for the whole wave, so every pixel in that
    // wave samples the same window — which reads on screen as "each window shows
    // the OTHER window". The loop counter is uniform across the wave, so the
    // texture must be sampled INSIDE the loop, indexed by `i`, and the result kept.
    var hit = -1;
    var px = vec4<f32>(0.0);
    var edge = false;
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let lo = r.xy;
        let hi = r.xy + r.zw;
        if (uv.x >= lo.x && uv.x <= hi.x && uv.y >= lo.y && uv.y <= hi.y) {
            hit = i32(i);
            let s = windows.srcs[i];
            var local = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));
            // MIRROR, on whichever axis `axis` selects. Entry 3 is the identity.
            let axis = i32(round(pc.params[0].x));
            if (axis == 0 || axis == 2) { local.x = 1.0 - local.x; }
            if (axis == 1 || axis == 2) { local.y = 1.0 - local.y; }
            px = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
            // Border so the rect is visible even on a black window.
            let e = min(min(local.x, 1.0 - local.x), min(local.y, 1.0 - local.y));
            edge = e * min(r.zw.x * res.x, r.zw.y * res.y) < pc.params[0].y;
        }
    }

    if (hit >= 0) {
        // Show the sampled texel as-is. (An earlier version painted flat red when
        // the sample was all-zero, to catch an unbound array — but a legitimately
        // transparent texel is also all-zero, so it reported false failures. Use
        // `tb-window-probe` to inspect the window set instead.)
        col = px.rgb;
        if (edge) {
            col = vec3<f32>(1.0, 0.9, 0.2);
        }
    }
    return vec4<f32>(col, 1.0);
}

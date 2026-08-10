// TB — `targets.persist`, pass 2 of 2: draw the desktop with the trail under it.
//
// This pass is ordinary. It samples `trail` like any other intermediate — the
// persistence is entirely a property of the TARGET, not of the reader, which is
// the point: nothing here has to know it is looking at an accumulator.
//
// It owns the world band (`windows: "world"`), so it draws the windows itself,
// over the trail. Otherwise the engine would draw them on top and the ghost would
// only ever be visible outside every window — which happens to be where a trail
// mostly is, so the bug would look almost right.
//
// @prop glow    float default=0.75 min=0.0 max=2.0 step=0.01 label="Trail glow" group="Trail"

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var trail: texture_2d<f32>;

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let glow = pc.params[0].x;

    var col = mix(
        vec3<f32>(0.02, 0.03, 0.05),
        vec3<f32>(0.05, 0.03, 0.08),
        0.5 + 0.5 * sin((uv.x + uv.y) * 2.0 + t * 0.1),
    );
    // The ghost, added rather than composited, so it reads as light left behind.
    col = col + textureSampleLevel(trail, samp, uv, 0.0).rgb * glow;

    // The band, back to front, over it.
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let hi = r.xy + r.zw;
        if (uv.x < r.x || uv.x > hi.x || uv.y < r.y || uv.y > hi.y) {
            continue;
        }
        let s = windows.srcs[i];
        let local = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));
        let pp = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
        let a = clamp(pp.a, 0.0, 1.0) * windows.attrs[i].y;
        col = col * (1.0 - a) + pp.rgb * a;
    }

    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

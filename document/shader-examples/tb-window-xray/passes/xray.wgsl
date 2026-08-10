// TB STRESS 13 — "tb-window-xray", pass 2/2 (AFTER-CONTENT, rects + textures).
//
// Edge-detects each window's OWN texture (Sobel over 8 taps of the bindless
// array) and draws it as glowing cyan wireframe over a darkened window body.
// Windows become blueprints — instantly readable as "the shader is sampling the
// real window texture", and it exercises MULTI-TAP bindless sampling rather than
// the single tap `tb-window-mirror` uses.
//
// Multi-tap matters for the worker: every tap is a read of a CLIENT dmabuf, so
// this is the bundle that would show sync problems first if those buffers were
// imported into a second device without honouring the client's acquire fence —
// tearing or half-updated window content inside the wireframe.
//
// Dead array -> flat red body (never a silent no-op).
//
// `style` is the shipped exercise for `choices=`: a discrete prop whose values are
// branches the shader actually takes, so the settings panel offers the branch NAMES
// rather than "1.00" on a slider. It is still one float in one param slot — the
// choice list changes how it is edited, not how it is marshalled.
//
// @prop glow  float default=1.0 min=0.0 max=3.0  step=0.01 label="Edge glow"  group="X-ray"
// @prop body  float default=0.18 min=0.0 max=1.0 step=0.01 label="Body level" group="X-ray"
// @prop style int   default=0 choices="Wireframe,Solid,Edges only"  label="Style" group="X-ray"

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

// `w` MUST be the caller's loop counter — see the hazard note in fs_main. Named
// for that: it is a wave-uniform window index, never a per-pixel one.
fn lum_at(w: u32, uv: vec2<f32>) -> f32 {
    let p = textureSampleLevel(win_tex[w], samp, uv, 0.0).rgb;
    return dot(p, vec3<f32>(0.299, 0.587, 0.114));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    var col = textureSampleLevel(scene, samp, uv, 0.0).rgb;

    // NON-UNIFORM INDEXING HAZARD: `win_tex` is a `binding_array` and naga emits
    // no `NonUniform` decoration, so indexing it with a PER-PIXEL value is
    // undefined — the driver resolves one lane's index for the whole wave and
    // every pixel samples the same window ("each window shows the OTHER window").
    // Sample INSIDE the loop, indexed by the wave-uniform loop counter.
    // `lum_at` is only ever called with the loop counter, so its index is uniform.
    var hit = -1;
    var body = vec3<f32>(0.0);
    var dead = false;
    var edge = 0.0;
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let lo = r.xy;
        let hi = r.xy + r.zw;
        if (uv.x >= lo.x && uv.x <= hi.x && uv.y >= lo.y && uv.y <= hi.y) {
            hit = i32(i);
            let s = windows.srcs[i];
            let local = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));
            let tex_uv = s.xy + local * s.zw;
            let base = textureSampleLevel(win_tex[i], samp, tex_uv, 0.0);
            dead = base.r + base.g + base.b + base.a <= 0.0;
            body = base.rgb * pc.params[0].y;
            // Sobel in texture space, one texel of the window's own crop.
            let d = s.zw / max(r.zw * res, vec2<f32>(1.0));
            let tl = lum_at(i, tex_uv + vec2<f32>(-d.x, -d.y));
            let tm = lum_at(i, tex_uv + vec2<f32>( 0.0, -d.y));
            let tr = lum_at(i, tex_uv + vec2<f32>( d.x, -d.y));
            let ml = lum_at(i, tex_uv + vec2<f32>(-d.x,  0.0));
            let mr = lum_at(i, tex_uv + vec2<f32>( d.x,  0.0));
            let bl = lum_at(i, tex_uv + vec2<f32>(-d.x,  d.y));
            let bm = lum_at(i, tex_uv + vec2<f32>( 0.0,  d.y));
            let br = lum_at(i, tex_uv + vec2<f32>( d.x,  d.y));
            let gx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
            let gy = (bl + 2.0 * bm + br) - (tl + 2.0 * tm + tr);
            edge = clamp(sqrt(gx * gx + gy * gy) * 1.6, 0.0, 1.0);
        }
    }

    if (hit >= 0) {
        if (dead) {
            col = vec3<f32>(0.8, 0.05, 0.05);
        } else {
            let wire = vec3<f32>(0.2, 1.0, 1.0) * edge * pc.params[0].x;
            let style = i32(round(pc.params[0].z));
            if (style == 1) {
                col = body;                     // Solid: the darkened body alone
            } else if (style == 2) {
                col = wire;                     // Edges only: no body at all
            } else {
                col = body + wire;              // Wireframe: both
            }
        }
    }
    return vec4<f32>(col, 1.0);
}

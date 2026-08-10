// TB STRESS 14 — "tb-parallax-lights", pass 2/2 (AFTER-CONTENT, needs window-rects).
//
// Lights the WINDOWS from the same emitters the background pass drew. Each
// window is treated as a flat panel with a soft bevelled edge: the bevel gives
// it a normal that tilts outward near the border, so a light to the left of a
// window catches its LEFT edge and rims it. Plus a broad diffuse wash by
// proximity and a Blinn specular sheen that sweeps as the emitter moves.
//
// WHY THIS BUNDLE IS THE INTERESTING ONE FOR THE WORKER PLAN
// ---------------------------------------------------------
// Neither pass is told where the lights are. Both compute `light_pos(i, time)`
// from their OWN push. On the inline path they share a clock and agree. If the
// before band is ever paced by the worker while this band runs in the
// compositor, the two clocks diverge — and the highlight on a window will point
// at a place the glowing orb ISN'T. The mismatch is directly visible: sight
// along the line from the orb to the rim highlight it is supposed to cause.
//
// That makes this the acceptance test for the clock-coherence rule in
// SHADER_PIPELINE_WORKER.md: bands must not assume a shared frame clock unless
// offload is disabled, and any bundle that derives shared geometry from `time`
// is exactly what breaks when they do.
//
// It also stays honest about window-rect lag: the lighting is anchored to the
// rects, so a rect that trails the window shows up as light sliding off the
// panel while you drag it.
//
// @prop intensity float default=1.10 min=0.0  max=3.0  step=0.05  label="Light intensity" group="Parallax lights"
// @prop reach     float default=0.45 min=0.05 max=1.5  step=0.01  label="Light reach"     group="Parallax lights"
// @prop bevel     float default=0.014 min=0.002 max=0.08 step=0.002 label="Bevel width"   group="Parallax lights"
// @prop sheen     float default=0.55 min=0.0  max=2.0  step=0.05  label="Specular sheen"  group="Parallax lights"

#import lights::light_pos
#import lights::light_tint
#import lights::light_falloff

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,  // 0 = intensity, reach, bevel, sheen
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
    let aspect = vec2<f32>(res.x / max(res.y, 1.0), 1.0);

    let intensity = pc.params[0].x;
    let reach     = pc.params[0].y;
    let bevel     = pc.params[0].z;
    let sheen     = pc.params[0].w;

    var col = textureSample(scene, samp, uv).rgb;
    let n = min(windows.count, 256u);

    // Front-most window covering this pixel.
    var hit = -1;
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let lo = r.xy;
        let hi = r.xy + r.zw;
        if (uv.x >= lo.x && uv.x <= hi.x && uv.y >= lo.y && uv.y <= hi.y) {
            hit = i32(i);
        }
    }
    if (hit < 0) {
        return vec4<f32>(col, 1.0);
    }

    let r = windows.rects[u32(hit)];
    let local = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));

    // Distance to the nearest edge, in aspect-corrected UV units.
    let ex = min(local.x, 1.0 - local.x) * r.zw.x * aspect.x;
    let ey = min(local.y, 1.0 - local.y) * r.zw.y;
    let edge = min(ex, ey);

    // Bevel normal: flat in the middle, tilting OUTWARD over the last `bevel`.
    let b = clamp(edge / max(bevel, 0.0001), 0.0, 1.0);
    var tilt = vec2<f32>(0.0);
    if (ex < ey) {
        tilt = vec2<f32>(select(1.0, -1.0, local.x < 0.5), 0.0);
    } else {
        tilt = vec2<f32>(0.0, select(1.0, -1.0, local.y < 0.5));
    }
    let nrm = normalize(vec3<f32>(tilt * (1.0 - b) * 1.35, 1.0));
    let view = vec3<f32>(0.0, 0.0, 1.0);

    // Accumulate every emitter.
    var lit = vec3<f32>(0.0);
    for (var i = 0u; i < 3u; i = i + 1u) {
        let lp = light_pos(i, t);
        let delta = (lp - uv) * aspect;
        let dist = length(delta);
        // Lights float slightly above the desktop plane.
        let ldir = normalize(vec3<f32>(delta, 0.30));
        let ndl = max(dot(nrm, ldir), 0.0);
        let fall = light_falloff(dist, reach);
        let tint = light_tint(i);

        // Broad wash + rim term (the bevel is what makes the edge catch).
        lit = lit + tint * fall * (0.22 + 0.95 * ndl);

        // Blinn specular sheen, strongest on the bevel facing the light.
        let h = normalize(ldir + view);
        let spec = pow(max(dot(nrm, h), 0.0), 42.0);
        lit = lit + tint * spec * fall * sheen;
    }

    // Additive, so window content stays readable underneath.
    col = col + lit * intensity * 0.5;
    return vec4<f32>(col, 1.0);
}

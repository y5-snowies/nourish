// TB STRESS 14 — "tb-parallax-lights", pass 1/2 (BEFORE-CONTENT).
//
// A real parallax background — three star layers drifting at different rates
// against the world pan (`pan_flow.xy`) — with three bright EMITTER artifacts
// moving through it. The emitters are drawn here; the after-content pass lights
// the windows from the same emitters (see lib/lights.wgsl).
//
// Offload class: this pass alone is worker-eligible (no `needs`, no
// post-composite input). The `lit` pass is not — so the bundle is BEFORE-BAND,
// and it is the realistic shape of an effect someone would actually ship.
//
// @prop density   float default=0.55 min=0.0 max=1.0  step=0.01 label="Star density" group="Parallax lights"
// @prop emitsize  float default=0.045 min=0.01 max=0.2 step=0.005 label="Emitter size" group="Parallax lights"

#import lights::light_pos
#import lights::light_tint

struct Push {
    res_zoom_time: vec4<f32>,     // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,          // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,  // 0.x = density, 0.y = emitter size
};
var<immediate> pc: Push;

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// One star layer: a jittered grid, `depth` scales both the parallax shift and
// the brightness so nearer layers move more and shine brighter.
fn star_layer(uv: vec2<f32>, pan: vec2<f32>, depth: f32, cells: f32, density: f32) -> f32 {
    let p = uv * cells + pan * depth * cells;
    let cell = floor(p);
    let f = fract(p);
    let r = hash2(cell);
    if (r > density) { return 0.0; }
    let c = vec2<f32>(hash2(cell + vec2<f32>(1.7, 9.2)), hash2(cell + vec2<f32>(4.1, 2.3)));
    let d = length(f - c);
    return smoothstep(0.28, 0.0, d) * (0.35 + 0.65 * r) * depth;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let aspect = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let density = pc.params[0].x;
    let esize = pc.params[0].y;

    // World pan drives the parallax; normalised so a full-screen pan shifts the
    // nearest layer by roughly one screen.
    let pan = pc.pan_flow.xy / res;

    // Deep gradient + faint nebula so the layers have something to sit on.
    var col = mix(vec3<f32>(0.015, 0.02, 0.05), vec3<f32>(0.05, 0.03, 0.09), uv.y);
    let neb = sin(uv.x * 3.0 + t * 0.05) * cos(uv.y * 2.4 - t * 0.04);
    col = col + vec3<f32>(0.03, 0.02, 0.06) * (0.5 + 0.5 * neb);

    // Three parallax star layers.
    var s = 0.0;
    s = s + star_layer(uv, pan, 0.25, 18.0, density);
    s = s + star_layer(uv, pan, 0.55, 34.0, density);
    s = s + star_layer(uv, pan, 1.00, 60.0, density);
    col = col + vec3<f32>(0.85, 0.90, 1.00) * s * 0.55;

    // The emitters — bright cores with a wide soft halo.
    for (var i = 0u; i < 3u; i = i + 1u) {
        let lp = light_pos(i, t);
        let d = length((uv - lp) * aspect);
        let core = smoothstep(esize, 0.0, d);
        let halo = smoothstep(esize * 7.0, 0.0, d);
        col = col + light_tint(i) * (core * 1.6 + halo * 0.32);
    }

    return vec4<f32>(col, 1.0);
}

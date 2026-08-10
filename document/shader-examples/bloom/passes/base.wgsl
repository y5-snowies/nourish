// bloom pass 1/5 — "base": render the raw background scene into the `scene`
// target. No inputs; just the engine Push. A drifting nebula + SOFT round stars,
// with a handful of deliberately HDR-bright stars so the later bright-pass has
// obvious sources to bloom. Fragment-only: the engine pairs the fullscreen vertex.
//
// Binding ABI (see document/SHADER_PIPELINE.md): `pc.res_zoom_time.xy` is the
// CURRENT target's pixel size; `@builtin(position) frag` is its pixel coord.
//
// @prop speed   float default=0.30 min=0.0 max=2.0 step=0.01 label="Drift speed" group="Bloom"
// @prop density float default=0.55 min=0.0 max=1.0 step=0.01 label="Star density" group="Bloom"

struct Push {
    res_zoom_time: vec4<f32>,     // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,          // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,        // x = lock_amount, y = alpha
    params: array<vec4<f32>, 2>,  // @prop slots: 0 = speed, 1 = density
};
var<immediate> pc: Push;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}
fn hash22(p: vec2<f32>) -> vec2<f32> {
    return fract(sin(vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)))) * 43758.5453);
}

// One layer of soft round stars on a jittered grid. `grid` sets density/size;
// `bright_boost` HDR-lifts a rare few so they bloom hard.
fn star_layer(uv: vec2<f32>, grid: f32, t: f32, density: f32) -> vec3<f32> {
    let g = uv * grid;
    let cell = floor(g);
    let f = fract(g);
    var acc = vec3<f32>(0.0);
    // Sample the 3x3 neighbourhood so stars near cell edges still render round.
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let c = cell + vec2<f32>(f32(x), f32(y));
            let seed = hash21(c);
            if (seed > 1.0 - density * 0.25) {
                let pos = vec2<f32>(f32(x), f32(y)) + hash22(c + 1.0);
                let d = length(f - pos);
                let twinkle = 0.6 + 0.4 * sin(t * 2.0 + seed * 30.0);
                // Rare HDR-bright stars (seed high) → strong bloom sources.
                let hdr = 1.0 + step(0.985, hash21(c + 7.0)) * 8.0;
                let glow = smoothstep(0.09, 0.0, d) * twinkle * hdr;
                let tint = mix(vec3<f32>(0.7, 0.8, 1.0), vec3<f32>(1.0, 0.9, 0.75), seed);
                acc = acc + tint * glow;
            }
        }
    }
    return acc;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let t = pc.res_zoom_time.w;
    let speed = pc.params[0].x;   // @prop speed
    let density = pc.params[0].y; // @prop density
    var uv = frag.xy / res;
    uv.x = uv.x * (res.x / max(res.y, 1.0)); // aspect-correct

    // Nebula backdrop: cheap layered value noise, dark blue-violet.
    let drift = t * speed * 0.03;
    let n = hash21(floor((uv + drift) * 6.0)) * 0.5 + hash21(floor((uv - drift) * 12.0)) * 0.5;
    let nebula = mix(vec3<f32>(0.01, 0.02, 0.06), vec3<f32>(0.05, 0.03, 0.11), n);

    // Two star layers at different scales for depth.
    let stars = star_layer(uv, 14.0, t, density) + star_layer(uv, 30.0, t, density) * 0.6;

    return vec4<f32>(nebula + stars, 1.0);
}

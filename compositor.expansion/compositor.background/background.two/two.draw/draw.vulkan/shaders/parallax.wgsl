// Parallax space background — native Vulkan SDR fragment shader (WGSL → SPIR-V
// via naga at build time). A faithful port of the GLES `spacev3.frag`; runs in
// a real VkPipeline so the Vulkan shader path is exercised. Uniforms arrive as
// push constants (3 engine vec4 + 4 params vec4 = 7×vec4 = 112 bytes).
//
// NOTE: the value-noise `hash()` here is a driver-stable integer/bit-mix hash
// (Dave Hoskins' "hash without sine"), NOT the GLES `fract(sin(...))` hash. The
// sine hash diverges badly under range reduction at large coordinates on some
// Vulkan drivers, which showed up as rectangular "boxes" in the cloud/nebula
// noise. This hash is well-distributed regardless of driver, so the artifact is
// gone; the cloud pattern differs slightly from the GLES reference by design.

struct Push {
    res_zoom_time: vec4<f32>,        // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,             // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,           // x = lock_amount, y = alpha
    params: array<vec4<f32>, 4>,     // shader-authored @prop values (16 floats)
};
var<immediate> pc: Push;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), f.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), f.x),
        f.y,
    );
}
fn fbm(p_in: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var p = p_in;
    for (var i = 0; i < 5; i = i + 1) {
        v = v + a * noise(p);
        p = p * 2.0;
        a = a * 0.5;
    }
    return v;
}

fn draw_planet(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
               light_side: vec3<f32>, dark_side: vec3<f32>, light_dir: vec2<f32>,
               band_freq: f32) -> vec3<f32> {
    let pp = uv - center;
    let d = length(pp) - radius;
    let mask = smoothstep(0.004, -0.004, d);
    if (mask <= 0.0) { return col; }
    let lit = smoothstep(-radius * 0.6, radius * 0.6, dot(pp, light_dir));
    var base = mix(dark_side, light_side, lit);
    if (band_freq > 0.0) {
        let band = sin(pp.y * band_freq + center.x * 3.0) * 0.5 + 0.5;
        let band_noise = fbm(pp * 15.0) * 0.15;
        base = mix(base, base * 0.75, smoothstep(0.2, 0.8, band + band_noise));
    }
    let rim = smoothstep(radius * 0.5, radius, length(pp));
    let rim_lit = smoothstep(-radius * 0.2, radius, dot(pp, light_dir));
    let atmosphere = light_side * rim * rim_lit * 0.5;
    return mix(col, base + atmosphere, mask);
}

fn galaxy(uv: vec2<f32>, c: vec2<f32>, rot: f32, scale: vec2<f32>) -> f32 {
    var p = uv - c;
    let s = sin(rot);
    let co = cos(rot);
    p = vec2<f32>(co * p.x - s * p.y, s * p.x + co * p.y) / scale;
    let r2 = dot(p, p);
    return exp(-r2 * 6.0) * 0.6 + exp(-r2 * 45.0) * 0.4;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let frag = in.pos.xy;
    let res = pc.res_zoom_time.xy;
    let zoom = pc.res_zoom_time.z;
    let time = pc.res_zoom_time.w;
    let pan_in = pc.pan_flow.xy;
    let flow = pc.pan_flow.zw;
    let lock_amount = pc.lock_alpha.x;
    let alpha = pc.lock_alpha.y;
    // @prop-driven knobs (see shader.builtin): drift / density / nebula, plus the
    // vignette amount / radius / softness (slots 3..5).
    let drift_speed = pc.params[0].x;
    let star_density = pc.params[0].y;
    let nebula = pc.params[0].z;
    let vignette = pc.params[0].w;
    let vig_radius = pc.params[1].x;
    let vig_softness = pc.params[1].y;

    var uv = (frag - 0.5 * res) / res.y;
    // screen_uv is zoom-independent so the vignette frames the display, not the
    // world; the scene uv below is divided by zoom as usual.
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    var col = mix(vec3<f32>(0.01, 0.015, 0.04), vec3<f32>(0.04, 0.02, 0.09), frag.y / res.y);

    let neb_uv = uv * 1.5 + pan * 0.0002 + flow * 0.0003 + vec2<f32>(time * 0.01, time * 0.005) * drift_speed;
    let n = fbm(neb_uv);
    let n2 = fbm(neb_uv * 2.5 - vec2<f32>(time * 0.015, time * 0.015) * drift_speed);
    col = col + mix(vec3<f32>(0.25, 0.05, 0.35), vec3<f32>(0.05, 0.20, 0.45), n) * pow(n, 1.8) * 0.5 * nebula;
    col = col + vec3<f32>(0.1, 0.3, 0.4) * pow(n2, 3.0) * 0.25 * nebula;

    for (var i = 1; i <= 3; i = i + 1) {
        let depth = f32(i) * 0.5;
        let sp = uv * (45.0 / depth) + pan * 0.001 * depth;
        let id = floor(sp);
        let fp = fract(sp) - 0.5;
        let h = hash(id);
        if (h > 1.0 - 0.04 * star_density) {
            let twink = 0.5 + 0.5 * sin(time * 1.5 + h * 50.0);
            let dd = length(fp);
            let star_col = mix(vec3<f32>(0.7, 0.9, 1.0), vec3<f32>(1.0, 0.85, 0.7), fract(h * 133.7));
            let glow = smoothstep(0.06, 0.0, dd) + smoothstep(0.2, 0.0, dd) * 0.3;
            col = col + star_col * glow * twink / depth;
        }
    }

    {
        let drift = -flow * 0.0007 + vec2<f32>(time * 0.12 * drift_speed, 0.0);
        let p = uv * vec2<f32>(1.8, 12.0) + drift;
        let id = floor(p);
        let f = fract(p) - 0.5;
        let h = hash(id);
        if (h > 0.86) {
            let streak = smoothstep(0.5, 0.0, abs(f.y) * 5.0) * smoothstep(0.5, 0.0, abs(f.x) * 1.1);
            col = col + vec3<f32>(0.45, 0.65, 1.0) * streak * (h - 0.86) * 3.5;
        }
    }

    col = draw_planet(col, uv, vec2<f32>(-0.65, 0.30) - pan * 0.00015, 0.07,
                      vec3<f32>(0.85, 0.85, 0.90), vec3<f32>(0.18, 0.18, 0.22),
                      normalize(vec2<f32>(1.0, 0.3)), 0.0);
    col = draw_planet(col, uv, vec2<f32>(0.70, 0.15) - pan * 0.00030, 0.13,
                      vec3<f32>(0.35, 0.65, 0.55), vec3<f32>(0.08, 0.15, 0.12),
                      normalize(vec2<f32>(-0.6, 0.4)), 0.0);
    col = draw_planet(col, uv, vec2<f32>(-0.40, -0.30) - pan * 0.00055, 0.22,
                      vec3<f32>(0.90, 0.60, 0.35), vec3<f32>(0.15, 0.05, 0.08),
                      normalize(vec2<f32>(0.7, 0.5)), 15.0);

    // Lock-screen transition.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    if (l > 0.001) {
        let drift = time * 0.003;
        var lcol = mix(vec3<f32>(0.004, 0.006, 0.018), vec3<f32>(0.010, 0.015, 0.040),
                       clamp(uv.y * 0.5 + 0.5, 0.0, 1.0));
        let band_axis = dot(uv, normalize(vec2<f32>(0.6, -0.8))) + 0.55;
        let band_shape = exp(-band_axis * band_axis * 4.0);
        let band_tex = fbm(uv * 1.3 + vec2<f32>(drift, -2.0));
        lcol = lcol + mix(vec3<f32>(0.04, 0.05, 0.09), vec3<f32>(0.07, 0.06, 0.11), band_tex)
                * band_shape * band_tex * 0.5;

        // Deep field: countless tiny, dim, motionless stars (layer 2 redshifted).
        for (var i = 1; i <= 2; i = i + 1) {
            let dens = select(95.0, 55.0, i == 1);
            let thr = select(0.992, 0.980, i == 1);
            let sp = uv * dens + pan * 0.0002 * f32(i);
            let id = floor(sp);
            let fp = fract(sp) - 0.5;
            let h = hash(id);
            if (h > thr) {
                let dd = length(fp);
                let core = smoothstep(0.12, 0.0, dd);
                let sc = select(vec3<f32>(0.45, 0.52, 0.68), vec3<f32>(0.45, 0.30, 0.26), i == 2);
                lcol = lcol + sc * core * select(0.60, 0.35, i == 2);
            }
        }

        lcol = lcol + vec3<f32>(0.16, 0.15, 0.21) * galaxy(uv, vec2<f32>(0.52, 0.34) + drift, 0.6, vec2<f32>(0.13, 0.045)) * 0.55;
        lcol = lcol + vec3<f32>(0.13, 0.13, 0.19) * galaxy(uv, vec2<f32>(-0.58, -0.22) + drift, -0.3, vec2<f32>(0.09, 0.030)) * 0.45;
        lcol = lcol + vec3<f32>(0.12, 0.11, 0.17) * galaxy(uv, vec2<f32>(0.05, -0.40) + drift, 1.2, vec2<f32>(0.05, 0.020)) * 0.40;
        let vig = smoothstep(1.25, 0.15, length(uv));
        lcol = lcol * mix(0.30, 1.0, vig);
        col = mix(col, lcol, l);
    }

    // Optional vignette (slots 3..5): darken toward the edges when amount > 0.
    // Evaluated in screen space (zoom-independent) with knob-driven radius /
    // softness so the framing stays consistent as the world zooms.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

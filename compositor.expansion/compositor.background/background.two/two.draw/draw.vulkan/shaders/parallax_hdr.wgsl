// HDR parallax space background (M5). A WGSL port of the SDR parallax shader
// that renders the same scene but grades it for HDR: the diffuse base sits at
// the SDR reference white, while highlights (stars, planet rims, nebula cores)
// are extended above it into the HDR range for tasteful "pop", plus a subtle
// extra micro-star layer. Output is BT.2020 + PQ. Used ONLY on the HDR path.
//
// The value-noise `hash()` is the same driver-stable integer/bit-mix hash used
// by the SDR variant (NOT a `fract(sin(...))` sine hash), so the cloud/nebula
// noise has no rectangular artifacts under Vulkan.

// Matches draw.vulkan's HdrPush (8×vec4 = 128 bytes).
struct Push {
    res_zoom_time: vec4<f32>,    // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,         // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,       // x = lock_amount, y = alpha
    params: array<vec4<f32>, 4>, // shader-authored @prop values (16 floats)
    hdr: vec4<f32>,              // x = sdr_white_nits, y = max_nits, z/w reserved
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

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}
fn rec709_to_bt2020(c: vec3<f32>) -> vec3<f32> {
    let r = dot(vec3<f32>(0.627404, 0.329283, 0.043313), c);
    let g = dot(vec3<f32>(0.069097, 0.919540, 0.011362), c);
    let b = dot(vec3<f32>(0.016391, 0.088013, 0.895595), c);
    return vec3<f32>(r, g, b);
}
fn pq_encode(nits: vec3<f32>) -> vec3<f32> {
    let m1 = 0.1593017578125;
    let m2 = 78.84375;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let y = clamp(nits / 10000.0, vec3<f32>(0.0), vec3<f32>(1.0));
    let ym1 = pow(y, vec3<f32>(m1));
    return pow((vec3<f32>(c1) + c2 * ym1) / (vec3<f32>(1.0) + c3 * ym1), vec3<f32>(m2));
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

    var uv = (frag - 0.5 * res) / res.y;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    var col = mix(vec3<f32>(0.01, 0.015, 0.04), vec3<f32>(0.04, 0.02, 0.09), frag.y / res.y);

    let neb_uv = uv * 1.5 + pan * 0.0002 + flow * 0.0003 + vec2<f32>(time * 0.01, time * 0.005);
    let n = fbm(neb_uv);
    let n2 = fbm(neb_uv * 2.5 - vec2<f32>(time * 0.015, time * 0.015));
    col = col + mix(vec3<f32>(0.25, 0.05, 0.35), vec3<f32>(0.05, 0.20, 0.45), n) * pow(n, 1.8) * 0.5;
    col = col + vec3<f32>(0.1, 0.3, 0.4) * pow(n2, 3.0) * 0.25;

    // Star layers (slightly punchier than SDR so they read as HDR highlights).
    for (var i = 1; i <= 3; i = i + 1) {
        let depth = f32(i) * 0.5;
        let sp = uv * (45.0 / depth) + pan * 0.001 * depth;
        let id = floor(sp);
        let fp = fract(sp) - 0.5;
        let h = hash(id);
        if (h > 0.96) {
            let twink = 0.5 + 0.5 * sin(time * 1.5 + h * 50.0);
            let dd = length(fp);
            let star_col = mix(vec3<f32>(0.7, 0.9, 1.0), vec3<f32>(1.0, 0.85, 0.7), fract(h * 133.7));
            let glow = smoothstep(0.06, 0.0, dd) + smoothstep(0.2, 0.0, dd) * 0.3;
            col = col + star_col * glow * twink / depth;
        }
    }

    // Extra fine micro-star layer — subtle added detail the HDR range can show.
    {
        let sp = uv * 140.0 + pan * 0.0006;
        let id = floor(sp);
        let fp = fract(sp) - 0.5;
        let h = hash(id + 7.0);
        if (h > 0.988) {
            let dd = length(fp);
            let tw = 0.6 + 0.4 * sin(time * 2.3 + h * 80.0);
            col = col + vec3<f32>(0.8, 0.9, 1.0) * smoothstep(0.08, 0.0, dd) * tw * 0.5;
        }
    }

    // Shooting streaks.
    {
        let drift = -flow * 0.0007 + vec2<f32>(time * 0.12, 0.0);
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

    // Lock-screen transition (same as SDR).
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
        lcol = lcol + vec3<f32>(0.16, 0.15, 0.21) * galaxy(uv, vec2<f32>(0.52, 0.34) + drift, 0.6, vec2<f32>(0.13, 0.045)) * 0.55;
        lcol = lcol + vec3<f32>(0.13, 0.13, 0.19) * galaxy(uv, vec2<f32>(-0.58, -0.22) + drift, -0.3, vec2<f32>(0.09, 0.030)) * 0.45;
        let vig = smoothstep(1.25, 0.15, length(uv));
        lcol = lcol * mix(0.30, 1.0, vig);
        col = mix(col, lcol, l);
    }

    // ── HDR grade ───────────────────────────────────────────────────────────
    // `col` is the SDR display look. Decode to linear, put the diffuse base at
    // the reference white, and push highlights above it into the HDR range so
    // stars/rims/cores "pop" — capped at a tasteful multiple of reference white.
    let sdr_white = max(pc.hdr.x, 1.0);
    let max_nits = max(pc.hdr.y, sdr_white);
    var lin = srgb_to_linear(max(col, vec3<f32>(0.0)));
    let luma = dot(lin, vec3<f32>(0.2627, 0.6780, 0.0593));
    let peak = min(max_nits, sdr_white * 4.0);
    let hi = smoothstep(0.5, 1.0, luma);
    var nits = lin * (sdr_white * 1.05) + lin * (hi * hi) * (peak - sdr_white);
    nits = rec709_to_bt2020(max(nits, vec3<f32>(0.0)));
    let enc = pq_encode(nits);
    // Premultiplied alpha (matches the SDR background blend).
    return vec4<f32>(enc * alpha, alpha);
}

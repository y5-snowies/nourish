// Built-in background: "Fiery Galaxy" — a calm, unobtrusive parallax scene of a
// small cluster of molten worlds glowing in deep space. A companion to the stock
// space parallax: same quiet, dark, low-contrast mood, same Push/`@prop` contract.
// The heat is an accent, never a glare — thin veins of slow-flowing lava over a
// dark crust.
//
// Design notes (kept deliberately restful):
//   * Near-black sky with a faint warm ember haze.
//   * Three worlds at different depths; each a dark basalt crust webbed with a
//     network of glowing cracks that drift and pulse very slowly. The crust spins
//     on its axis, driven by the drift-speed knob.
//   * The lava is emissive, so the shadowed side still breathes a low ember
//     glow instead of going black — but the peak stays gentle.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Drift & spin" group="Fiery"
// @prop star_density float default=1.0 min=0.0 max=2.0 label="Star density" group="Fiery"
// @prop ember float default=1.0 min=0.0 max=2.0 label="Ember intensity" group="Fiery"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Fiery"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Fiery"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Fiery"

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

// `hash` is a pure function of two fract lanes: the vec3's z lane always
// duplicates x, since both are `fract(p.x * 0.1031)`. Splitting the lane setup
// out lets `noise` compute its four corner lanes once and share them across all
// four taps. Every operand and operation order below is unchanged, so the output
// is bit-identical to the vec3 form this replaced.
fn hash_lane(x: f32, y: f32) -> f32 {
    let p3 = vec3<f32>(x, y, x);
    let d = dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract(((x + d) + (y + d)) * (x + d));
}
fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * 0.1031);
    return hash_lane(q.x, q.y);
}
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    // One vec4 mul+fract for all four corner lanes, each still computed exactly as
    // the four separate `hash` calls did: fract(i.x*K), fract((i.x+1)*K),
    // fract(i.y*K), fract((i.y+1)*K).
    let q = fract(vec4<f32>(i.x, i.x + 1.0, i.y, i.y + 1.0) * 0.1031);
    return mix(
        mix(hash_lane(q.x, q.z), hash_lane(q.y, q.z), f.x),
        mix(hash_lane(q.x, q.w), hash_lane(q.y, q.w), f.x),
        f.y,
    );
}
// Octave count for `fbm`. The loader rewrites this literal to the `@optimized`
// value when the world's Optimized toggle is on, and naga folds it through the
// loop below — one source, two compiled variants, nothing to keep in step.
const FBM_OCTAVES: i32 = 5;   // @optimized 2
// The reference walk's amplitude sum. `fbm` renormalises to it so a shortened
// walk keeps this scene's brightness instead of coming out dim. At the reference
// count the factor is exactly 1.0 — both values are exact binary fractions — so
// the reference output is bit-identical to the fixed-5-octave version.
const FBM_REF_SUM: f32 = 0.96875;
fn fbm(p_in: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var norm = 0.0;
    var p = p_in;
    for (var i = 0; i < FBM_OCTAVES; i = i + 1) {
        v = v + a * noise(p);
        norm = norm + a;
        p = p * 2.0;
        a = a * 0.5;
    }
    return v * (FBM_REF_SUM / norm);
}

fn stars(uv: vec2<f32>, pan: vec2<f32>, time: f32, density: f32) -> vec3<f32> {
    var col = vec3<f32>(0.0);
    for (var i = 1; i <= 3; i = i + 1) {
        let depth = f32(i) * 0.5;
        let sp = uv * (42.0 / depth) + pan * 0.001 * depth;
        let id = floor(sp);
        let fp = fract(sp) - 0.5;
        let h = hash(id + f32(i) * 17.0);
        if (h > 1.0 - 0.03 * density) {
            let twink = 0.6 + 0.4 * sin(time * 0.7 + h * 40.0);
            let dd = length(fp);
            let tint = mix(vec3<f32>(0.95, 0.88, 0.78), vec3<f32>(1.0, 0.82, 0.6), fract(h * 51.7));
            col = col + tint * smoothstep(0.05, 0.0, dd) * twink / (depth * 2.4);
        }
    }
    return col;
}

// The hero: a dark crust webbed with glowing, slowly flowing lava. `ember`
// scales the heat; the network comes from ridged contours of a drifting fbm.
fn draw_molten(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
               rot: f32, time: f32, ember: f32) -> vec3<f32> {
    let pp = (uv - center) / radius;
    let r2 = dot(pp, pp);
    let mask = smoothstep(1.0, 1.0 - 0.02 / radius, r2);
    if (mask <= 0.0) { return col; }

    let z = sqrt(max(1.0 - r2, 0.0));
    let n = vec3<f32>(pp, z);
    // A dim external key just to give the crust some form; the lava supplies the
    // real light, so even the night side keeps a low ember glow.
    let light_dir = normalize(vec3<f32>(-0.5, 0.4, 0.35));
    let day = clamp(dot(n, light_dir), 0.0, 1.0);

    // Foreshortened surface coords; `rot` spins the axis (drift-speed knob).
    let lon = atan2(n.x, n.z) * 0.3183099 + rot;
    let lat = asin(clamp(n.y, -1.0, 1.0)) * 0.6366198;
    let s = vec2<f32>(lon, lat) * 3.0;

    // Dark basalt crust (stays dark — the lava is the only real brightness).
    let grain = fbm(s * 3.0);
    var crust = mix(vec3<f32>(0.018, 0.013, 0.013), vec3<f32>(0.05, 0.035, 0.032), grain);
    crust = crust * (0.3 + 0.55 * day);

    // Slowly flowing lava: THIN glowing veins tracing the iso-contours of a
    // drifting fbm, so most of the crust stays dark and only the cracks glow.
    let flow = vec2<f32>(time * 0.010, -time * 0.006);
    let f1 = fbm(s * 1.9 + flow);
    let f2 = fbm(s * 3.8 - flow * 1.2 + 5.0);
    let vein1 = smoothstep(0.035, 0.0, abs(f1 - 0.5));
    let vein2 = smoothstep(0.02, 0.0, abs(f2 - 0.5)) * 0.6;
    let pulse = 0.75 + 0.25 * sin(time * 0.5 + f1 * 6.2832);
    let heat = clamp(vein1 + vein2, 0.0, 1.0) * pulse * ember;
    let lava = mix(vec3<f32>(0.55, 0.11, 0.02), vec3<f32>(1.0, 0.72, 0.26), heat) * heat;

    // A soft ember bloom hugging the veins so they bleed a little light — kept low.
    let bloom = smoothstep(0.09, 0.0, abs(f1 - 0.5)) * 0.16 * ember;
    var body = crust + vec3<f32>(0.5, 0.17, 0.04) * bloom + lava;

    // Warm heat-haze rim, a thin glow along the lit limb.
    let rim = pow(1.0 - z, 3.2);
    body = body + vec3<f32>(0.65, 0.24, 0.06) * rim * (0.3 + 0.7 * smoothstep(-0.1, 0.8, day)) * 0.5 * ember;

    return mix(col, body, mask);
}

// A banded gas giant: turbulent latitude bands, soft terminator, thin rim.
fn draw_banded(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
               rot: f32, ca: vec3<f32>, cb: vec3<f32>) -> vec3<f32> {
    let pp = (uv - center) / radius;
    let r2 = dot(pp, pp);
    let mask = smoothstep(1.0, 1.0 - 0.02 / radius, r2);
    if (mask <= 0.0) { return col; }
    let z = sqrt(max(1.0 - r2, 0.0));
    let n = vec3<f32>(pp, z);
    let day = clamp(dot(n, normalize(vec3<f32>(-0.5, 0.4, 0.35))), 0.0, 1.0);
    let term = smoothstep(0.0, 0.55, day);
    let lat = asin(clamp(n.y, -1.0, 1.0));
    let lon = atan2(n.x, n.z) * 0.3183099 + rot;
    let turb = fbm(vec2<f32>(lon * 2.5, lat * 3.5)) * 0.6;
    let band = sin(lat * 8.0 + turb * 4.0) * 0.5 + 0.5;
    var surf = mix(ca, cb, band);
    surf = surf * (0.85 + 0.3 * fbm(vec2<f32>(lon * 3.0 + rot, lat * 6.0)));
    var body = mix(surf * 0.18, surf, term);
    let iz = 1.0 - z;
    let rim = iz * iz * iz;
    body = body + ca * rim * smoothstep(-0.1, 0.7, day) * 0.5;
    return mix(col, body, mask);
}

// A small, self-luminous star: soft additive corona plus a granular, pulsing core.
fn draw_star(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
             time: f32, core: vec3<f32>, edge: vec3<f32>) -> vec3<f32> {
    let pp = (uv - center) / radius;
    let d = length(pp);
    let glow = exp(-d * d * 2.2) * 0.4 + smoothstep(1.7, 0.0, d) * 0.08;
    var body = col + edge * glow;
    let disc = smoothstep(1.0, 0.8, d);
    let gran = 0.82 + 0.35 * fbm(pp * 6.0 + time * 0.06);
    let pulse = 0.9 + 0.1 * sin(time * 0.9);
    body = mix(body, core * gran * pulse, disc);
    return body;
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

    let drift = pc.params[0].x;
    let star_density = pc.params[0].y;
    let ember = pc.params[0].z;
    let vignette = pc.params[0].w;
    let vig_radius = pc.params[1].x;
    let vig_softness = pc.params[1].y;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    // Pan convention: on-screen content tracks the camera as -pan_in on both axes.
    let pan = vec2<f32>(pan_in.x, pan_in.y);

    // Very dark sky, the faintest warm lift toward the bottom.
    var col = mix(vec3<f32>(0.02, 0.014, 0.016), vec3<f32>(0.035, 0.014, 0.012), frag.y / res.y);

    // Faint warm ember haze drifting slowly, concentrated low.
    let haze_uv = uv * 1.2 + pan * 0.00016 + flow * 0.0003 + vec2<f32>(time * 0.008, time * 0.004) * drift;
    let h1 = fbm(haze_uv);
    col = col + mix(vec3<f32>(0.14, 0.05, 0.02), vec3<f32>(0.06, 0.03, 0.04), h1) * pow(h1, 2.2) * 0.32 * ember;

    col = col + stars(uv, pan, time, star_density);

    // Drift-driven axial spin; farther worlds turn a touch faster and parallax more.
    let spin = time * drift * 0.05;

    // A varied trio: the molten hero, a dark ember-banded gas giant, and a small
    // burning star.
    col = draw_molten(col, uv, vec2<f32>(0.5, -0.34) - pan * 0.00030, 0.36, spin, time, ember);
    col = draw_banded(col, uv, vec2<f32>(-0.58, 0.24) - pan * 0.00060, 0.17, spin * 1.2,
                      vec3<f32>(0.24, 0.10, 0.035), vec3<f32>(0.07, 0.028, 0.022));
    col = draw_star(col, uv, vec2<f32>(0.1, 0.45) - pan * 0.00095, 0.05, time,
                    vec3<f32>(1.0, 0.72, 0.34), vec3<f32>(0.85, 0.36, 0.1));

    // Lock-screen ease: cool and settle into a stiller, darker night.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.012, 0.006, 0.006), l);

    // Guarded because the default is off: the length() and the smoothstep would
    // otherwise run on every pixel only to be multiplied by 1.0. The guard also
    // makes a 0 softness safe — that smoothstep divides by zero, and
    // mix(1.0, NaN, 0.0) is NaN, not 1.0.
    let vig_amount = clamp(vignette, 0.0, 1.0);
    if (vig_amount > 0.0) {
        let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
        col = col * mix(1.0, vig, vig_amount);
    }
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    var outc = col;
    // An `if`, not `select` — `select` evaluates both arms, so these three
    // pow()s ran on every pixel even with the flag off (the default). The branch is
    // on a push constant, so it is uniform across the whole draw.
    if (pc.lock_alpha.z > 0.5) {
        outc = pow(max(outc, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

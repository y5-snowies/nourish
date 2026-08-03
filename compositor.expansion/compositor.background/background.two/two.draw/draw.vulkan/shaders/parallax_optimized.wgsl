// Parallax space background — the OPTIMIZED variant of `parallax.wgsl`.
//
// Same scene, same composition, same 112-byte `Push` (the pipeline is built with
// `push.len()`, so the layout may NOT drift from the reference). Selected per
// world by the "Optimized" toggle in Settings → Current World; the reference
// shader stays the default, and the picker and lock screen never reach this file
// because they are separate worlds with their own `Two` slot.
//
// It exists because the reference shader is ALU-bound on a weak GPU — a
// Raspberry Pi's V3D in particular — and the cost is almost entirely value
// noise: two 5-octave `fbm()` calls per pixel is 10 `noise()` = 40 `hash()`.
//
// What changed, and only this:
//   * `fbm2()` — ONE 2-octave noise walk yielding two fields. `x` is the usual
//     sum; `y` is octave 2 alone, standing in for the second nebula layer the
//     reference pays a whole second 5-octave fbm for. 10 noise evaluations per
//     pixel become 2, and it is amplitude-renormalised to the reference's 0.96875
//     ceiling so the nebula matches in brightness rather than coming out dim.
//     It also serves the banded planet's surface detail, which is scaled by 0.15
//     and thresholded through a `smoothstep` — high octaves never survived that.
//   * `pow18()` — `pow(n, 1.8)` as `n*n` times a linear correction, trading two
//     SFU ops for two ALU ops at ~5% error on a term that is scaled by 0.5.
//   * `draw_planet()` bails on the SQUARED distance, so the ~all pixels outside
//     every planet pay a dot and a compare instead of a sqrt and a smoothstep.
//
// The noise field is therefore a DIFFERENT INSTANCE of the same statistics: the
// clouds sit in the same places (octave 1 is untouched) but are visibly smoother
// — two octaves where the reference has five. That is the deliberate trade. The
// scene itself — three star layers, the streaks, all three planets, the
// lock-screen transition — is structurally identical to the reference.
//
// The `hash()` is deliberately left as the reference's float Hoskins hash rather
// than swapped for an integer bit-mix. An integer hash is fewer ops on paper, but
// V3D lowers 32-bit `imul` into multiple `umul24` instructions, so the win is not
// guaranteed on the exact hardware this variant exists for. The octave reduction
// above is architecture-neutral; revisit the hash only with a measurement in hand.

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
// The reference's 5-octave amplitudes sum to 0.96875. Each reduced walk below is
// scaled back up to that ceiling so the nebula keeps the reference's brightness
// and contrast instead of coming out dim.
const FBM_CEIL: f32 = 0.96875;

/// One 2-octave noise walk, two fields.
///   x = the full sum (amplitudes .5 .25 → 0.75), the nebula body.
///   y = octave 2 alone (.25), the finer field that stands in for the reference's
///       second, 2.5×-scaled fbm. It is cubed and scaled to a faint tint at the
///       call site, so a single octave carries it.
/// TWO `noise()` evaluations where the reference spends ten. This is the whole
/// budget: the clouds are smoother than the reference's five octaves, which is the
/// deliberate trade this variant exists to make.
fn fbm2(p_in: vec2<f32>) -> vec2<f32> {
    var v = 0.0;
    var w = 0.0;
    var a = 0.5;
    var p = p_in;
    for (var i = 0; i < 2; i = i + 1) {
        let s = a * noise(p);
        v = v + s;
        // The loop unrolls, so this folds away — no runtime branch.
        if (i > 0) { w = w + s; }
        p = p * 2.0;
        a = a * 0.5;
    }
    return vec2<f32>(v * (FBM_CEIL / 0.75), w * (FBM_CEIL / 0.25));
}

/// `pow(n, 1.8)` without the log2/exp2 pair: `n*n` times a linear correction for
/// `n^-0.2`. Within ~5% across [0.2, 1] — far tighter than plain `n*n`, which runs
/// 13% dark through the mid-tones — and the nebula it shapes is scaled by 0.5 over
/// a near-black sky, so that error is nowhere near visible.
fn pow18(n: f32) -> f32 {
    return n * n * (1.42 - 0.42 * n);
}

fn draw_planet(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
               light_side: vec3<f32>, dark_side: vec3<f32>, light_dir: vec2<f32>,
               band_freq: f32) -> vec3<f32> {
    let pp = uv - center;
    // Bail on the SQUARED distance. Almost every pixel on screen is outside every
    // planet, and this way those pixels pay one dot and one compare instead of a
    // sqrt, a subtract and a smoothstep — three times over. `r`, `lit_dot` and the
    // mask are then only paid by pixels that actually touch the disc, and the
    // `mask <= 0.0` test the reference needs is subsumed by this one.
    let outer = radius + 0.004;
    if (dot(pp, pp) > outer * outer) { return col; }
    let r = length(pp);
    let d = r - radius;
    let mask = smoothstep(0.004, -0.004, d);
    let lit_dot = dot(pp, light_dir);
    let lit = smoothstep(-radius * 0.6, radius * 0.6, lit_dot);
    var base = mix(dark_side, light_side, lit);
    if (band_freq > 0.0) {
        let band = sin(pp.y * band_freq + center.x * 3.0) * 0.5 + 0.5;
        // `fbm2().x` is the old `fbm_lo` now that the walk is two octaves — same
        // octaves, same renormalisation. The unused `y` lane folds away.
        let band_noise = fbm2(pp * 15.0).x * 0.15;
        base = mix(base, base * 0.75, smoothstep(0.2, 0.8, band + band_noise));
    }
    let rim = smoothstep(radius * 0.5, radius, r);
    let rim_lit = smoothstep(-radius * 0.2, radius, lit_dot);
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
    // Pan convention: horizontal tracks the camera as -pan_in. Vertical is inverted
    // by default here (the built-in parallax reads better this way); the per-world
    // "Invert pan Y" toggle flips it back from this baseline.
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    var col = mix(vec3<f32>(0.01, 0.015, 0.04), vec3<f32>(0.04, 0.02, 0.09), frag.y / res.y);

    // The nebula is the whole reason this variant exists. One `fbm2` walk (3
    // noise evaluations) replaces the reference's two 5-octave fbm calls (10), and
    // the second layer comes out of the same taps instead of a second walk. Still
    // guarded on `nebula`, which zeroes both terms.
    if (nebula > 0.0) {
        let neb_uv = uv * 1.5 + pan * 0.0002 + flow * 0.0003 + vec2<f32>(time * 0.01, time * 0.005) * drift_speed;
        let nn = fbm2(neb_uv);
        let n = nn.x;
        let n2 = nn.y;
        col = col + mix(vec3<f32>(0.25, 0.05, 0.35), vec3<f32>(0.05, 0.20, 0.45), n) * pow18(n) * 0.5 * nebula;
        // n2*n2*n2, not pow(n2, 3.0): two multiplies instead of a log2/exp2 pair,
        // and more accurate for it.
        col = col + vec3<f32>(0.1, 0.3, 0.4) * (n2 * n2 * n2) * 0.25 * nebula;
    }

    // At `star_density` 0 the threshold sits at 1.0, which `hash` never exceeds —
    // the three layers would hash and discard. Skip them outright.
    if (star_density > 0.0) {
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
        let band_tex = fbm2(uv * 1.3 + vec2<f32>(drift, -2.0)).x;
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
    // softness so the framing stays consistent as the world zooms. Guarded because
    // the default is off, and the `length()` + `smoothstep` would otherwise run per
    // pixel only to be multiplied by 1.0. The guard also keeps a 0 softness safe:
    // that smoothstep divides by zero, and `mix(1.0, NaN, 0.0)` is NaN, not 1.0.
    let vig_amount = clamp(vignette, 0.0, 1.0);
    if (vig_amount > 0.0) {
        let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
        col = col * mix(1.0, vig, vig_amount);
    }
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode so a non-sRGB scanout
    // buffer shows the brighter, preview-matching look. Off = raw values. An `if`,
    // not `select` — `select` evaluates both arms, so the three `pow`s ran on every
    // pixel even with the flag off (the default). The branch is on a push constant,
    // so it is uniform across the draw.
    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

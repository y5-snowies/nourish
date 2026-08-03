// Built-in background: "Leafy Drift" — the *inside* companion to Leafy Galaxy.
// You're within the leafy world's air: a calm green light with an array of leaves
// drifting on the wind, scattered like the star field and carried by the canvas —
// panning gusts them along. Same quiet mood, same Push/`@prop` contract.
//
// Design notes (kept deliberately restful):
//   * A soft green gradient lit from the upper-left (sun through the canopy),
//     with faint slanted light shafts and a whisper of dapple — never busy.
//   * Four parallax layers of leaf sprites, seeded and jittered per grid cell
//     (density knob). Near leaves are large, crisp and fast; far leaves are
//     small, slow and hazed into the green air. Each leaf is aligned roughly
//     to the wind, tumbles in 3D (foreshortening as it turns over), flutters,
//     carries a curved spine, a midrib and faint side veins, and casts a soft
//     shadow onto the air behind it.
//   * The wind carries them diagonally over time; the canvas pan and its
//     velocity (flow) push and tilt them, so scrolling the workspace stirs
//     the leaves.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop wind float default=1.0 min=0.0 max=6.0 label="Wind speed" group="Drift"
// @prop leaf_density float default=1.0 min=0.0 max=2.0 label="Leaf density" group="Drift"
// @prop lushness float default=1.0 min=0.0 max=2.0 label="Lushness" group="Drift"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Drift"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Drift"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Drift"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 4>,
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
const FBM_OCTAVES: i32 = 4;   // @optimized 2
// The reference walk's amplitude sum. `fbm` renormalises to it so a shortened
// walk keeps this scene's brightness instead of coming out dim. At the reference
// count the factor is exactly 1.0 — both values are exact binary fractions — so
// the reference output is bit-identical to the fixed-4-octave version.
const FBM_REF_SUM: f32 = 0.9375;
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

// Signed distance-ish field of a leaf silhouette in local space: points along
// ±y, a gently curved spine (`bend`), width tapering to the tips and a touch
// teardrop-heavy at the base. Negative inside. `xs` (the spine-relative x) is
// what the caller shades veins against.
fn leaf_field(p: vec2<f32>, bend: f32) -> vec2<f32> {
    let y = p.y;
    let xs = p.x - bend * (y * y - 0.33);          // spine curves like a real leaf
    let halfw = 0.44 * (1.0 - y * y) * (1.0 - 0.28 * y);
    return vec2<f32>(abs(xs) - halfw, xs);
}

// One parallax layer of wind-blown leaves. `i` = 0 is the nearest layer: large,
// crisp, fast; higher layers recede — smaller, slower, hazed toward the air.
// `flow` is the pan-velocity (flow_offset) that gusts and tilts the leaves;
// `pan` translates them by depth. `px` is one screen pixel in uv units, used
// for analytic edge anti-aliasing (no derivatives, so it stays preview-safe).
fn leaf_layer(col_in: vec3<f32>, uv: vec2<f32>, pan: vec2<f32>, flow: vec2<f32>,
              time: f32, i: i32, wind: f32, density: f32, lush: f32, px: f32) -> vec3<f32> {
    var col = col_in;
    let depth = 1.0 + f32(i) * 0.75;
    let fog = clamp((depth - 1.0) / 2.6, 0.0, 1.0) * 0.55;   // aerial haze, far layers
    let detail = 1.0 - fog;                                   // veins fade with distance
    let gust = 1.0 + 0.4 * sin(time * 0.3 + f32(i) * 1.7);
    // Wind carries leaves down-and-across; near layers move (and pan) the most.
    let wdir = vec2<f32>(0.9, -0.45);
    let wvel = wind * 0.06 / depth;
    // Gusting is INTEGRATED, not multiplied into the clock. Writing the offset as
    // `time * gust` makes the velocity `gust + time * gust'` — a term that grows
    // without bound, so the leaves accelerate the longer the session runs. The
    // antiderivative of `1 + 0.4*sin(0.3t + φ)` is `t - (0.4/0.3)*cos(0.3t + φ)`,
    // which gusts the speed by ±40% around `wvel` and never drifts.
    let carry = time - 1.3333333 * cos(time * 0.3 + f32(i) * 1.7);
    let move_ = wdir * (carry * wvel)
              + pan * (0.0022 / depth)
              + flow * (0.0028 / depth);
    let scale = 4.6 * depth;                                  // far cells are smaller
    let coord = uv * scale + move_;
    let base = floor(coord);
    // One pixel in leaf-cell units, for edge AA and the soft shadow feather.
    let cellpx = px * scale;
    // A gust cants every leaf the same way when the canvas is thrown.
    let tilt = clamp((flow.x - flow.y) * 0.0008, -0.8, 0.8);
    // atan2(-0.45, 0.9) — `wdir` is a literal, but naga does not fold atan2, so
    // spelling the constant saves one transcendental per layer per pixel.
    let wang = -0.4636476;

    // Leaves are jittered off their cell centers, so scan the 3×3 neighbourhood
    // (same idea as the snowfall treeline) to let them cross cell borders.
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = base + vec2<f32>(f32(dx), f32(dy));
            let h = hash(c + f32(i) * 41.0);
            if (h <= 1.0 - 0.09 * density) { continue; }

            let ph = fract(h * 97.0);
            let ph2 = fract(h * 57.31);
            let size = 0.32 + 0.18 * fract(h * 7.77);
            // Jittered anchor + a slow positional flutter on the breeze.
            let jitter = (vec2<f32>(hash(c + 7.7), hash(c + 13.1)) - 0.5) * 0.55;
            // Gust rides the AMPLITUDE, not the clock — `time * gust` would ramp
            // this flutter's frequency without bound (see `carry` above).
            let sway = vec2<f32>(sin(time * 0.8 + h * 21.0),
                                 cos(time * 1.1 + h * 33.0)) * (0.04 * gust);
            let f = coord - (c + 0.5 + jitter) - sway;
            if (dot(f, f) > (size + 0.1) * (size + 0.1)) { continue; }

            // Orientation: aligned to the wind with a per-leaf spread, a slow
            // spin, a flutter, and the gust tilt.
            let ang = wang + (ph - 0.5) * 2.4
                    + time * 0.3 * (ph2 - 0.5)
                    + sin(time * 1.4 + h * 25.0) * (0.35 * gust)
                    + tilt;
            let ca = cos(-ang);
            let sa = sin(-ang);
            var lp = vec2<f32>(ca * f.x - sa * f.y, sa * f.x + ca * f.y) / size;

            // 3D tumble: the leaf turns over as it drifts, foreshortening its
            // width and catching the light when it faces us.
            let tum = time * (0.35 + 0.5 * ph2) / depth + h * 40.0;
            // `abs(cos)` has a CUSP wherever the leaf turns edge-on: the width
            // stops shrinking and starts growing in a single frame, which reads as
            // a snap. `sqrt(c² + k)` is the same curve with that corner rounded
            // over a window of ~sqrt(k); the two constants renormalise it back to
            // exactly [0, 1] so the widest and narrowest leaves are unchanged.
            let ct = cos(tum);
            let turn = (sqrt(ct * ct + 0.02) - 0.1414214) * 1.1513666;
            let facing = 0.34 + 0.66 * turn;
            lp.x = lp.x / facing;

            // Foreshorten the spine curve with the width, or edge-on leaves
            // exaggerate into banana slivers.
            let bend = (fract(h * 17.3) - 0.5) * 0.5 * mix(0.5, 1.0, facing);
            let fld = leaf_field(lp, bend);
            let aa = max(cellpx / (size * facing), 0.012) * (1.0 + 2.5 * fog);
            let m = smoothstep(aa, -aa, fld.x) * smoothstep(1.0 + aa, 1.0 - aa, abs(lp.y));

            // Soft contact shadow on the air behind, cast down-right of the
            // upper-left light. Evaluated at a world-space offset, wide feather.
            let fs = f - vec2<f32>(0.07, -0.055) * size;
            var sp = vec2<f32>(ca * fs.x - sa * fs.y, sa * fs.x + ca * fs.y) / size;
            sp.x = sp.x / facing;
            let sfld = leaf_field(sp, bend);
            let sh = smoothstep(0.18, -0.05, sfld.x) * smoothstep(1.15, 0.9, abs(sp.y));
            col = col * (1.0 - 0.24 * sh * (1.0 - m) * detail);

            if (m <= 0.0) { continue; }

            // Colour: mostly greens with a few golden/amber leaves for life.
            let phc = fract(h * 5.13);
            var lc = mix(vec3<f32>(0.10, 0.26, 0.11), vec3<f32>(0.27, 0.36, 0.13), phc)
                   * (0.7 + 0.5 * lush);
            lc = mix(lc, vec3<f32>(0.38, 0.25, 0.07), smoothstep(0.78, 0.88, ph) * 0.75);
            // Lit from the world's upper-left, brighter when facing us, with a
            // darker rim at the silhouette so the shape reads against the air.
            let lit = clamp(0.5 + dot(f / size, normalize(vec2<f32>(-0.6, 0.8))) * 0.45, 0.0, 1.0);
            lc = lc * (0.72 + 0.42 * lit) * (0.82 + 0.28 * facing);
            lc = lc * (1.0 - 0.20 * smoothstep(-0.16, 0.0, fld.x));
            // Veins: a midrib that stops short of the tips, and faint side veins
            // angled off the spine; both fade with distance.
            let rib = smoothstep(0.09, 0.015, abs(fld.y)) * smoothstep(1.0, 0.72, abs(lp.y));
            let vw = abs(fract(lp.y * 2.6 - abs(fld.y) * 1.6) - 0.5);
            let veins = smoothstep(0.10, 0.03, vw) * smoothstep(0.0, -0.12, fld.x)
                      * smoothstep(0.95, 0.55, abs(lp.y));
            lc = lc * (1.0 - 0.26 * rib * (0.5 + 0.5 * detail) - 0.10 * veins * detail);
            // Recede into the green air rather than turning transparent.
            lc = mix(lc, vec3<f32>(0.10, 0.17, 0.11), fog);
            col = mix(col, lc, m * (0.95 - 0.30 * fog));
        }
    }
    return col;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let frag = in.pos.xy;
    let res = pc.res_zoom_time.xy;
    let zoom = pc.res_zoom_time.z;
    let time = pc.res_zoom_time.w;
    let pan_in = pc.pan_flow.xy;
    let flow_in = pc.pan_flow.zw;
    let lock_amount = pc.lock_alpha.x;
    let alpha = pc.lock_alpha.y;

    let wind = pc.params[0].x;
    let density = pc.params[0].y;
    let lush = pc.params[0].z;
    let vignette = pc.params[0].w;
    let vig_radius = pc.params[1].x;
    let vig_softness = pc.params[1].y;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    // Pan convention: on-screen content tracks the camera as -pan_in on both axes.
    let pan = vec2<f32>(pan_in.x, pan_in.y);
    let flow = vec2<f32>(flow_in.x, -flow_in.y);
    let px = 1.0 / (res.y * zoom);

    // Soft green air with a gentle light from the upper-left (sun through leaves).
    var col = mix(vec3<f32>(0.03, 0.075, 0.045), vec3<f32>(0.06, 0.14, 0.08), clamp(screen_uv.y * 0.5 + 0.5, 0.0, 1.0));
    let glow = exp(-((screen_uv.x + 0.5) * (screen_uv.x + 0.5) + (screen_uv.y - 0.4) * (screen_uv.y - 0.4)) * 1.4);
    col = col + vec3<f32>(0.05, 0.10, 0.05) * glow * (0.6 + 0.4 * lush);
    // Slanted light shafts falling from the glow, drifting slowly on the wind.
    let sdir = normalize(vec2<f32>(-0.55, 0.83));
    let sx = dot(uv + pan * 0.0002, vec2<f32>(-sdir.y, sdir.x));
    let shafts = fbm(vec2<f32>(sx * 3.0 - time * 0.02 * (0.5 + 0.5 * wind), 1.7));
    col = col + vec3<f32>(0.045, 0.08, 0.04) * (shafts * shafts) * glow * (0.5 + 0.5 * lush);
    // A very soft canopy dapple so the empty air isn't flat.
    col = col + vec3<f32>(0.03, 0.06, 0.03) * fbm(uv * 1.4 + pan * 0.0003 + vec2<f32>(time * 0.02 * wind, 0.0)) * 0.4;

    // Leaves, far → near (near layer drawn last, on top). At density 0 every cell
    // fails its `h <= 1.0` test, so all four layers would hash 9 cells apiece and
    // discard every one — 36 wasted hashes per pixel. Skip them outright.
    if (density > 0.0) {
        col = leaf_layer(col, uv, pan, flow, time, 3, wind, density, lush, px);
        col = leaf_layer(col, uv, pan, flow, time, 2, wind, density, lush, px);
        col = leaf_layer(col, uv, pan, flow, time, 1, wind, density, lush, px);
        col = leaf_layer(col, uv, pan, flow, time, 0, wind, density, lush, px);
    }

    // Lock-screen ease: still the air and deepen the green toward dusk.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.006, 0.016, 0.01), l);

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
    // Ordered-noise dither hides banding in the smooth gradients (8-bit target);
    // applied post-encode, where the quantisation actually happens.
    outc = outc + (hash(frag) - 0.5) * (1.5 / 255.0);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

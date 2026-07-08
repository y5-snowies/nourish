// Built-in background: "Dusk Dunes" — the warm, low-frequency cousin of Misty
// Ridges. Instead of jagged peaks, smooth rolling dune crests recede toward a big
// soft sun sinking into a dusty gold-and-violet sky. Nothing moves fast.
//
// Design notes (kept deliberately restful):
//   * Same far → near layered-silhouette idea as the ridges, but the crests are
//     built from low-frequency, gently smoothed fBm so they roll rather than spike.
//   * The palette is warm: pale dusty gold at the horizon into a dusk violet
//     overhead, dune fills from sunlit sand (far) to deep umber shadow (near).
//   * One large soft sun disc rests on the horizon, its glow bleeding into the sky
//     and warming the crests that overlap it. `warmth` shifts the whole mood.
//   * Very slow drift; the canvas pan slides the dunes by depth.
//   * A few distant birds drift across the dusk sky, flapping on a discrete
//     sprite-frame clock (see sunset_birds.wgsl for the shared silhouette+flap).
//     They are drawn before the dunes so nearer crests occlude them at the skyline.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Drift" group="Dunes"
// @prop warmth float default=1.0 min=0.0 max=2.0 label="Warmth" group="Dunes"
// @prop dune_height float default=1.0 min=0.2 max=2.0 label="Dune height" group="Dunes"
// @prop sun_size float default=1.0 min=0.3 max=2.0 label="Sun size" group="Dunes"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Dunes"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Dunes"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Dunes"
// @prop bird_density float default=1.0 min=0.0 max=2.0 label="Bird count" group="Birds"
// @prop bird_speed float default=1.0 min=0.0 max=4.0 label="Bird drift" group="Birds"

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
// A softened, low-frequency fBm for rolling dune crests: only two octaves, weighted
// toward the base wave so the profile stays smooth and sinuous, not busy.
fn dune_fbm(p: vec2<f32>) -> f32 {
    return 0.72 * noise(p) + 0.28 * noise(p * 2.13 + vec2<f32>(5.2, 1.7));
}

fn dune_crest(x: f32, freq: f32, amp: f32, seed: f32) -> f32 {
    let n = dune_fbm(vec2<f32>(x * freq + seed, seed * 0.6));
    return amp * (n - 0.5) * 2.0;
}

// Distance from point `p` to the segment a→b (for the two wing strokes).
fn seg(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// One bird silhouette in local cell space `f` (centred, ~[-0.5,0.5]). `openness`
// (0 = wings down, 1 = wings up) is the current animation frame's wing pose; a
// distant gull is two shallow strokes meeting at the body, tips bent down a touch.
fn bird(f: vec2<f32>, openness: f32, size: f32) -> f32 {
    let p = f / size;
    if (dot(p, p) > 1.6) { return 0.0; }
    let span = 0.9;
    let lift = mix(-0.12, 0.5, openness);                 // wing-tip height by frame
    let tipL = vec2<f32>(-span, lift);
    let tipR = vec2<f32>(span, lift);
    let bend = vec2<f32>(0.0, -0.02);                     // slight body droop
    let d = min(seg(p, bend, tipL), seg(p, bend, tipR));
    return smoothstep(0.16, 0.06, d);
}

const N_DUNES: i32 = 4;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let frag = in.pos.xy;
    let res = pc.res_zoom_time.xy;
    let zoom = pc.res_zoom_time.z;
    let time = pc.res_zoom_time.w;
    let pan_in = pc.pan_flow.xy;
    let lock_amount = pc.lock_alpha.x;
    let alpha = pc.lock_alpha.y;

    let drift = pc.params[0].x;
    let warmth = pc.params[0].y;
    let dune_height = pc.params[0].z;
    let sun_size = pc.params[0].w;
    let vignette = pc.params[1].x;
    let vig_radius = pc.params[1].y;
    let vig_softness = pc.params[1].z;
    let bird_density = pc.params[1].w;
    let bird_speed = pc.params[2].x;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    let y = -uv.y;
    let x = uv.x;

    // Dusk sky: dusty gold at the horizon warming into a violet overhead. `warmth`
    // pushes the whole gradient toward amber.
    let horizon_col = mix(vec3<f32>(0.80, 0.55, 0.34), vec3<f32>(0.92, 0.62, 0.32), clamp(warmth - 0.5, 0.0, 1.0));
    let zenith_col = mix(vec3<f32>(0.26, 0.20, 0.34), vec3<f32>(0.34, 0.18, 0.28), clamp(warmth - 0.5, 0.0, 1.0));
    var col = mix(horizon_col, zenith_col, smoothstep(-0.05, 0.6, y));

    // The sun: a large soft disc resting just on the horizon, with a wide glow. Kept
    // behind the dunes (drawn first) so crests eclipse its lower edge.
    let sun_pos = vec2<f32>(0.10 - pan.x * 0.00006, 0.0);
    let sr = 0.16 * sun_size;
    let sd = length(vec2<f32>(x, y) - sun_pos);
    let disc = smoothstep(sr, sr - 0.02, sd);
    col = mix(col, vec3<f32>(1.0, 0.86, 0.58), disc);
    col = col + vec3<f32>(0.55, 0.34, 0.18) * exp(-sd * sd * 5.0) * (0.6 + 0.5 * warmth);
    col = col + vec3<f32>(1.0, 0.80, 0.5) * smoothstep(sr + 0.02, sr, sd) * 0.5;

    // Birds: a sparse flock drifting across the dusk sky, flapping on a discrete
    // sprite-frame clock (the same indexing a real atlas would use). Drawn here,
    // before the dunes, so nearer crests occlude any bird that dips to the skyline.
    let bp = vec2<f32>(x, y) * vec2<f32>(2.2, 4.0)
           + vec2<f32>(time * 0.05 * bird_speed + pan.x * 0.0004, pan.y * 0.0004);
    let bid = floor(bp);
    let bf = fract(bp) - 0.5;
    let hb = hash(bid + vec2<f32>(41.0, 7.0));
    if (hb > 1.0 - 0.16 * bird_density) {
        // Discrete flap: a 4-frame wing cycle (down → mid → up → mid), advanced by a
        // per-bird fps so the flock doesn't beat in unison.
        let frames = 4.0;
        let fps = 5.0 * (0.6 + 0.9 * fract(hb * 13.0));
        let frame = floor(time * fps + hb * 20.0) % frames;
        let openness = 1.0 - abs(frame - 2.0) * 0.5;        // 0,0.5,1,0.5 over the cycle
        let size = 0.16 + 0.10 * fract(hb * 47.0);
        let m = bird(bf + vec2<f32>(0.0, 0.12 * fract(hb * 91.0)), openness, size);
        // Keep the flock in the sky: above the horizon, thinning toward the top.
        let band = smoothstep(0.02, 0.14, y) * smoothstep(0.95, 0.28, y);
        col = mix(col, col * 0.14, m * band * 0.82);
    }

    // Dunes, far → near: smooth silhouettes tinted from sunlit sand to shadowed umber.
    for (var i = 0; i < N_DUNES; i = i + 1) {
        let t = f32(i) / f32(N_DUNES - 1);           // 0 = farthest, 1 = nearest
        let baseline = mix(0.03, -0.32, t);
        let amp = mix(0.035, 0.15, t) * dune_height;
        let freq = mix(0.7, 1.7, t);                 // low freq → rolling, not spiky
        let seed = 8.0 + f32(i) * 13.9;
        let parallax = mix(0.00010, 0.00080, t);
        let dx = x + pan.x * parallax + time * 0.004 * drift * (0.3 + t);
        let crest = baseline + dune_crest(dx, freq, amp, seed) + pan.y * parallax * 0.6;

        let cov = smoothstep(0.0, 0.006 / zoom, crest - y);

        // Sand tint with a lit crest / shadowed trough gradient. Farther dunes catch
        // more of the sun's warmth; nearer ones fall into umber shadow.
        let far_tint = vec3<f32>(0.86, 0.62, 0.38);
        let near_tint = vec3<f32>(0.26, 0.15, 0.12);
        var sand = mix(far_tint, near_tint, t);
        sand = sand * (0.80 + 0.28 * smoothstep(-0.35, 0.02, y - crest));   // lit crest
        sand = sand * (0.85 + 0.3 * warmth * (1.0 - t));                    // sun warmth
        // A little glow where a near dune overlaps the sun behind it (rim of light).
        let rim = smoothstep(sr + 0.12, sr, sd) * (1.0 - t);
        sand = sand + vec3<f32>(0.5, 0.32, 0.16) * rim * smoothstep(0.02, 0.0, abs(y - crest));
        col = mix(col, sand, cov);
    }

    // Lock-screen ease: sink toward a deep dusk.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.42 + vec3<f32>(0.02, 0.012, 0.014), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

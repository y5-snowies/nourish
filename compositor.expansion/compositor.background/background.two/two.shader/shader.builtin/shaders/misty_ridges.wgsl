// Built-in background: "Misty Ridges" — the classic layered-mountain calm.
// Several parallax ridgelines recede into a hazy horizon: each is a flat, tinted
// silhouette cut from 1D fBm, paler and higher as it goes back (aerial perspective),
// with soft mist pooling in the valleys between. A low sun glows behind them.
//
// Design notes (kept deliberately restful):
//   * The whole scene is drawn far → near. The sky is a vertical gradient from a
//     pale, hazy band at the horizon up to a deeper tint overhead.
//   * Each ridge layer is a 1D fBm profile h(x). A pixel is "inside" the layer when
//     it sits below that crest, so the layer is a flat filled silhouette. Nearer
//     layers are darker, taller, more detailed, and parallax harder with the pan.
//   * A thin mist band rides just above each crest so distant ridges dissolve into
//     the haze rather than ending on a hard line.
//   * Everything drifts very slowly sideways; the canvas pan slides the layers by
//     depth (near layers move most) and nudges them vertically a touch.
//   * A few distant birds drift across the hazy sky, flapping on a discrete
//     sprite-frame clock (see sunset_birds.wgsl for the shared silhouette+flap).
//     They are drawn before the ridges so nearer layers occlude them at the crest.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Drift" group="Ridges"
// @prop haze float default=1.0 min=0.0 max=2.0 label="Haze" group="Ridges"
// @prop ridge_height float default=1.0 min=0.2 max=2.0 label="Ridge height" group="Ridges"
// @prop sun float default=0.6 min=0.0 max=1.5 label="Sun glow" group="Ridges"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Ridges"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Ridges"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Ridges"
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

// A 1D ridgeline height (in "up" units) at horizontal position `x`. Built from fBm
// sampled along a line; centred around 0 so ridges rise and dip about the baseline.
fn ridgeline(x: f32, freq: f32, amp: f32, seed: f32) -> f32 {
    let n = fbm(vec2<f32>(x * freq + seed, seed * 0.7));
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

const N_RIDGES: i32 = 5;

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
    let haze = pc.params[0].y;
    let ridge_height = pc.params[0].z;
    let sun_amt = pc.params[0].w;
    let vignette = pc.params[1].x;
    let vig_radius = pc.params[1].y;
    let vig_softness = pc.params[1].z;
    let bird_density = pc.params[1].w;
    let bird_speed = pc.params[2].x;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    // Work in "up-positive" space so the horizon reads naturally.
    let y = -uv.y;
    let x = uv.x;

    // Sky: a pale, hazy horizon band rising to a deeper blue overhead.
    let horizon_col = mix(vec3<f32>(0.62, 0.66, 0.70), vec3<f32>(0.82, 0.80, 0.78), clamp(haze - 0.4, 0.0, 1.0));
    let zenith_col = vec3<f32>(0.24, 0.36, 0.52);
    var col = mix(horizon_col, zenith_col, smoothstep(-0.02, 0.55, y));

    // A low sun sitting near the horizon, glowing through the haze (behind ridges).
    let sun_pos = vec2<f32>(-0.22 - pan.x * 0.00008, 0.06);
    let sd = length((vec2<f32>(x, y) - sun_pos) * vec2<f32>(1.0, 1.2));
    col = col + vec3<f32>(0.55, 0.45, 0.32) * exp(-sd * sd * 26.0) * sun_amt;
    col = col + vec3<f32>(0.30, 0.24, 0.18) * exp(-sd * 3.2) * sun_amt * 0.6;

    // Birds: a sparse flock drifting across the hazy sky, flapping on a discrete
    // sprite-frame clock (the same indexing a real atlas would use). Drawn here,
    // before the ridges, so nearer layers occlude any bird that dips to a crest.
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
        let band = smoothstep(0.04, 0.16, y) * smoothstep(0.95, 0.28, y);
        col = mix(col, col * 0.18, m * band * 0.78);
    }

    // Ridges, far → near. Each is a flat silhouette below its fBm crest, tinted by
    // aerial perspective (far ≈ horizon haze, near ≈ deep cool blue).
    for (var i = 0; i < N_RIDGES; i = i + 1) {
        let t = f32(i) / f32(N_RIDGES - 1);          // 0 = farthest, 1 = nearest
        let baseline = mix(0.055, -0.30, t);
        let amp = mix(0.028, 0.17, t) * ridge_height;
        let freq = mix(1.1, 3.0, t);
        let seed = 3.0 + f32(i) * 11.37;
        let parallax = mix(0.00012, 0.00085, t);
        // Slow sideways drift plus the canvas pan (near layers slide most); a small
        // vertical parallax so panning up/down parts the layers a little.
        let rx = x + pan.x * parallax + time * 0.006 * drift * (0.25 + t);
        let crest = baseline + ridgeline(rx, freq, amp, seed) + pan.y * parallax * 0.6;

        // Soft 1px coverage edge; the layer fills everything below its crest.
        let cov = smoothstep(0.0, 0.006 / zoom, crest - y);

        // Mist pooling just above the crest — stronger for farther ridges and with
        // the haze knob — so distant ridges melt into the horizon.
        let mist = exp(-max(y - crest, 0.0) * mix(60.0, 22.0, t)) * (1.0 - t) * 0.7 * haze;
        col = mix(col, mix(col, horizon_col, 0.85), clamp(mist, 0.0, 1.0));

        // Aerial-perspective tint, plus a gentle vertical shade within the fill so
        // the silhouette isn't perfectly flat (lighter near the crest).
        let far_tint = mix(horizon_col, vec3<f32>(0.44, 0.52, 0.62), 0.5);
        let near_tint = vec3<f32>(0.10, 0.15, 0.24);
        var ridge_col = mix(far_tint, near_tint, t);
        ridge_col = ridge_col * (0.86 + 0.14 * smoothstep(-0.4, 0.05, y - crest));
        col = mix(col, ridge_col, cov);
    }

    // Lock-screen ease: dim toward a cool dusk.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.45 + vec3<f32>(0.01, 0.014, 0.02), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

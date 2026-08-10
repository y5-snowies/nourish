// dragons, pass 1 — the place. A dusk valley: sky, sun, cloud deck, three
// mountain ridges and a near treeline, rendered into an offscreen target that
// the band pass then draws the animal and the desktop over.
//
// It is its own pass rather than a block at the top of `band.wgsl` for one
// reason: `band.wgsl` already loops over every window and every dragon, and the
// landscape does not change with either. Computing it once into a target keeps
// the expensive pass doing only the work that actually varies per pixel.
//
// The realism here is atmospheric perspective, not detail. Each ridge is drawn
// further away than the last, and each one is mixed harder toward the colour of
// the sky BEHIND it — that single rule is what turns four grey shapes into
// distance. Contrast falls off with depth; it does not merely get darker.
//
// @prop daylight float default=0.55 min=0.05 max=1.50 step=0.01 label="Daylight"    group="World"
// @prop haze     float default=0.75 min=0.00 max=1.50 step=0.01 label="Atmosphere"  group="World"

#import dragons::common::d_fbm
#import dragons::common::d_ridge
#import dragons::common::d_noise

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 1>,
};
var<immediate> pc: Push;

const HORIZON: f32 = 0.660;
const SUN: vec2<f32> = vec2<f32>(0.735, 0.605);

// Height of ridge `layer` at aspect-space x. Higher octave counts on the near
// ridges only: the far ones are hazed into near-flatness anyway.
fn ridge_at(x: f32, scale: f32, seed: f32, oct: i32) -> f32 {
    return d_ridge(vec2<f32>(x * scale + seed * 7.3, seed * 1.77), oct);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let ar = res.x / res.y;
    let ax = uv.x * ar;

    let daylight = pc.params[0].x;
    let haze_amt = pc.params[0].y;

    // Panning the y5 world slides the landscape, and slides each layer by a
    // different amount — the parallax IS the depth cue, same as `mp-parallax`.
    let pan = pc.pan_flow.xy;

    // -- sky ---------------------------------------------------------------
    let up = clamp((HORIZON - uv.y) / HORIZON, 0.0, 1.0);        // 1 at zenith
    let zenith = vec3<f32>(0.030, 0.052, 0.115);
    let mid = vec3<f32>(0.140, 0.130, 0.190);
    let horizon_col = vec3<f32>(0.520, 0.260, 0.135);
    var sky = mix(mid, zenith, pow(up, 0.75));
    sky = mix(sky, horizon_col, pow(1.0 - up, 3.2));
    var col = sky * daylight;

    // Sun, low and hazy: a small disc inside a wide glow.
    let to_sun = vec2<f32>(ax - SUN.x * ar, uv.y - SUN.y);
    let sun_d = length(to_sun);
    col = col + vec3<f32>(1.00, 0.52, 0.22) * exp(-sun_d * 5.2) * 0.55 * daylight;
    col = col + vec3<f32>(1.00, 0.78, 0.48)
              * smoothstep(0.028, 0.012, sun_d) * 1.6 * daylight;

    // -- cloud deck ---------------------------------------------------------
    // Two layers at different rates; the lower one lit warm from beneath by the
    // sun, which is what a dusk deck actually does.
    if (uv.y < HORIZON) {
        let drift = vec2<f32>(t * 0.0055 + pan.x * 0.010, 0.0);
        let cp = vec2<f32>(ax * 1.30, (uv.y - 0.10) * 3.10) + drift;
        let dens = d_fbm(cp, 5);
        let band = smoothstep(0.02, 0.34, uv.y) * smoothstep(HORIZON, 0.30, uv.y);
        let cloud = smoothstep(0.46, 0.74, dens) * band;
        // Underlit edge: brighter where the cloud thins toward the sun.
        let lit = smoothstep(0.40, 0.68, dens) * exp(-sun_d * 2.1);
        let cloud_col = mix(vec3<f32>(0.115, 0.105, 0.140),
                            vec3<f32>(0.95, 0.55, 0.30), clamp(lit * 1.5, 0.0, 1.0));
        col = mix(col, cloud_col * daylight * 1.15, clamp(cloud * 0.92, 0.0, 1.0));

        let cp2 = vec2<f32>(ax * 2.60, (uv.y - 0.04) * 5.40) + drift * 2.2;
        let d2 = d_fbm(cp2, 4);
        let c2 = smoothstep(0.54, 0.80, d2) * smoothstep(0.0, 0.22, uv.y)
                 * smoothstep(0.52, 0.24, uv.y);
        col = mix(col, vec3<f32>(0.70, 0.42, 0.28) * daylight, c2 * 0.45);
    }

    // -- ridges -------------------------------------------------------------
    // Far to near. `haze` per layer is the whole trick: the far ridge is mixed
    // almost entirely into the sky colour at its own height, so it reads as
    // twenty miles off rather than as pale grey paint.
    let scales = array<f32, 3>(1.30, 2.35, 4.10);
    let seeds = array<f32, 3>(3.10, 8.70, 15.30);
    let bases = array<f32, 3>(0.545, 0.588, 0.632);
    let amps = array<f32, 3>(0.150, 0.115, 0.085);
    let hazes = array<f32, 3>(0.860, 0.620, 0.330);
    let paras = array<f32, 3>(0.006, 0.014, 0.030);
    let octs = array<i32, 3>(4, 5, 5);

    for (var k = 0; k < 3; k = k + 1) {
        let sx = ax + pan.x * paras[k];
        let h = ridge_at(sx, scales[k], seeds[k], octs[k]);
        let top = bases[k] - amps[k] * h;
        if (uv.y < top) {
            continue;
        }
        // Rock, darkening downward into the valley shadow.
        let into = clamp((uv.y - top) / 0.30, 0.0, 1.0);
        var rock = mix(vec3<f32>(0.052, 0.056, 0.072),
                       vec3<f32>(0.022, 0.024, 0.034), into);
        // Slope detail, and a warm edge on the crest facing the sun.
        let det = d_fbm(vec2<f32>(sx * 14.0 + seeds[k], uv.y * 22.0), 3);
        rock = rock * mix(0.82, 1.24, det);
        let crest = smoothstep(0.030, 0.0, uv.y - top);
        let sunside = smoothstep(0.9, 0.0, abs(sx - SUN.x * ar));
        rock = rock + vec3<f32>(0.85, 0.45, 0.22) * crest * sunside
                    * 0.55 * daylight;
        // Snow catching the last light, on the far high ridges only.
        if (k < 2) {
            let snow = smoothstep(0.055, 0.012, uv.y - top)
                     * smoothstep(0.45, 0.70, det);
            rock = mix(rock, vec3<f32>(0.62, 0.55, 0.58) * daylight, snow * 0.55);
        }
        // Atmospheric perspective: toward the sky colour at THIS height.
        let hz = clamp(hazes[k] * haze_amt, 0.0, 0.97);
        col = mix(rock, col, hz);
    }

    // -- valley floor and treeline -----------------------------------------
    let tree_top = 0.700 - 0.030 * d_ridge(vec2<f32>(ax * 9.0 + pan.x * 0.055, 4.4), 4);
    if (uv.y > tree_top) {
        let into = clamp((uv.y - tree_top) / 0.34, 0.0, 1.0);
        let grain = d_fbm(vec2<f32>(ax * 26.0 + pan.x * 0.06, uv.y * 30.0), 4);
        var ground = mix(vec3<f32>(0.030, 0.036, 0.030),
                         vec3<f32>(0.012, 0.014, 0.013), into);
        ground = ground * mix(0.75, 1.30, grain);
        col = mix(col, ground, 0.94);
    }

    // Mist pooling along the treeline, and a low haze band on the horizon.
    let mist = exp(-abs(uv.y - 0.690) * 26.0) * 0.30
             * (0.6 + 0.4 * d_fbm(vec2<f32>(ax * 3.0 + t * 0.012, 2.0), 3));
    col = col + vec3<f32>(0.34, 0.26, 0.28) * mist * haze_amt * daylight;

    // Slight vignette; keeps the eye in the middle where the dragons fly.
    let v = length((uv - vec2<f32>(0.5)) * vec2<f32>(ar, 1.0));
    col = col * (1.0 - 0.22 * smoothstep(0.45, 1.15, v));

    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}

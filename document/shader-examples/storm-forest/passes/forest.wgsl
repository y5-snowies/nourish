// storm-forest, pass 1/5 — the forest itself, drawn at half resolution.
//
// Layered conifer silhouettes receding into rain haze, blurred by passes 2 and 3
// into a depth-of-field backdrop.
//
// The ONE thing that makes this read as a forest is that every layer is DARKER
// than the sky and each layer is darker than the one behind it. Get that wrong —
// let the haze lift a distant layer above the sky it sits against — and the whole
// thing collapses into flat fog, which is exactly what it did the first time.
// So the layer colour is built by fogging a single near-black body toward the fog
// colour by depth, and never the other way round.
//
// It reads the SAME strike clock as the band pass, so when the bolt fires the
// canopy is lit on the same frame the bolt is drawn.
//
// @prop skyglow   float default=1.00 min=0.0 max=2.0  step=0.01 label="Sky brightness"        group="Forest"
// @prop hazedepth float default=0.45 min=0.0 max=1.0  step=0.01 label="Rain haze"             group="Forest"
// @prop density   float default=1.00 min=0.2 max=2.0  step=0.01 label="Tree density"          group="Forest"
// @prop period    float default=9.00 min=2.0 max=40.0 step=0.5  label="Seconds between bolts" group="Lightning"
// @prop flash     float default=1.00 min=0.0 max=2.5  step=0.01 label="Lightning brightness"  group="Lightning"
// @prop drift     float default=0.50 min=0.0 max=3.0  step=0.01 label="Fog drift"             group="Forest"

#import storm::quake::strike_at
#import storm::field::s_fbm
#import storm::field::s_ridge
#import storm::field::s_hash11
#import storm::field::flash_curve

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

const RIDGE_OCT: i32 = 4;   // @optimized 3
const FOG_OCT: i32 = 3;     // @optimized 2

/// 1 where this layer's trees are, 0 in the sky above them. `base` is the canopy
/// line in UV (down-positive), `amp` how far the spikes rise above it.
fn canopy(p: vec2<f32>, base: f32, amp: f32, freq: f32, seed: f32) -> f32 {
    let r = s_ridge(vec2<f32>(p.x * freq + seed * 31.0, seed * 7.0), RIDGE_OCT);
    let h = base - amp * r;
    return smoothstep(h - 0.008, h + 0.008, p.y);
}

/// Vertical trunk strokes breaking the canopy line, so the skyline is not one
/// continuous ridge. Added to the mask, not multiplied into it — multiplying
/// confines them to where the canopy already is, which draws nothing at all.
fn trunks(p: vec2<f32>, base: f32, freq: f32, seed: f32, thick: f32) -> f32 {
    let g = p.x * freq + seed * 13.0;
    let c = floor(g);
    let f = fract(g);
    let r = s_hash11(c + seed * 5.0);
    if (r < 0.45) {
        return 0.0;
    }
    let top = base - 0.030 - r * 0.110;
    let w = thick * (0.30 + 0.70 * r);
    let core = 1.0 - smoothstep(w, w * 1.9, abs(f - 0.5) * 2.0);
    return core * smoothstep(top - 0.010, top + 0.010, p.y);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let t = pc.res_zoom_time.w;

    let sky_amt = pc.params[0].x;
    let haze = pc.params[0].y;
    let density = pc.params[0].z;
    let period = pc.params[0].w;
    let flash_amt = pc.params[1].x;
    let drift = pc.params[1].y;

    let s = strike_at(t, period);
    let fl = flash_curve(t - s.x) * flash_amt;
    let bolt_x = fract(sin(s.y * 12.9898) * 43758.5453);
    let toward = 1.0 - smoothstep(0.0, 0.85, abs(uv.x - bolt_x));

    let pan = pc.pan_flow.xy * 0.00006;

    // --- sky --------------------------------------------------------------
    // Bright enough at the horizon that a black tree line has something to bite
    // against. This is the reference every layer below has to stay under.
    let horizon = smoothstep(0.80, 0.18, uv.y);
    var col = mix(
        vec3<f32>(0.018, 0.026, 0.046),
        vec3<f32>(0.082, 0.094, 0.112),
        horizon,
    ) * sky_amt;

    let cloud = s_fbm(vec2<f32>(uv.x * 2.2 + t * 0.010 * drift, uv.y * 3.4 - t * 0.004), FOG_OCT);
    col = col + vec3<f32>(0.030, 0.034, 0.044) * cloud * sky_amt * horizon;

    // The flash lights the cloud deck from behind, brightest near the bolt.
    let sky_lit = fl * (0.35 + 0.85 * toward) * (0.35 + 0.85 * (1.0 - uv.y));
    col = col + vec3<f32>(0.55, 0.66, 0.90) * sky_lit * (0.35 + 0.9 * cloud);

    // --- canopy layers, far to near ---------------------------------------
    let fog_col = vec3<f32>(0.062, 0.074, 0.090);
    let body = vec3<f32>(0.004, 0.009, 0.008);   // near-black wet conifer

    for (var i = 0; i < 4; i = i + 1) {
        let fi = f32(i);
        let d = fi / 3.0;                        // 0 = furthest, 1 = nearest
        let p = vec2<f32>(uv.x + pan.x * (0.25 + d * 2.2), uv.y);

        let base = 0.42 + d * 0.30;
        let amp = (0.075 + d * 0.160) * density;
        let freq = (5.5 - d * 3.2) * density;
        let m = canopy(p, base, amp, freq, fi + 1.0);
        let tr = trunks(p, base, (22.0 - d * 12.0) * density, fi + 4.0, 0.085);
        let mask = clamp(m + tr, 0.0, 1.0);

        // Fog only ever lifts a layer TOWARD the fog colour, and always less
        // than the layer behind it. Monotone by construction.
        let f = pow(1.0 - d, 1.4) * (0.30 + 0.70 * haze);
        var layer = mix(body, fog_col * 1.15, clamp(f, 0.0, 0.92));

        // Backlight: the flash rims the canopy facing the bolt. Kept small on
        // the near layers so they stay silhouettes.
        layer = layer + vec3<f32>(0.42, 0.52, 0.72) * fl * (0.08 + 0.45 * toward) * (1.0 - d * 0.75);

        col = mix(col, layer, mask);
    }

    // --- ground mist -------------------------------------------------------
    // One band at the very bottom, not a sheet per layer: stacking four of those
    // is what flattened the whole lower screen into fog last time.
    let mist_n = s_fbm(vec2<f32>(uv.x * 3.0 + t * 0.035 * drift, uv.y * 6.0 - t * 0.02 * drift), FOG_OCT);
    let mist = smoothstep(0.80, 1.05, uv.y) * (0.35 + 0.65 * mist_n) * haze;
    col = mix(col, fog_col * 1.10 + vec3<f32>(0.26, 0.33, 0.45) * fl * 0.5, clamp(mist * 0.75, 0.0, 1.0));

    // No sRGB encode here — this is an intermediate target.
    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}

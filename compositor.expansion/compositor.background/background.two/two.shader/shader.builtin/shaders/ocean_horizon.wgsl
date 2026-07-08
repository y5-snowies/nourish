// Built-in background: "Ocean Horizon" — a calm sea under a low sun. A soft sky
// gradient over a receding wave plane, the sun resting just above the waterline,
// and its light spilling down the water as a shimmering, broken glitter road.
// A few faraway sails sit on the horizon if you want company.
//
// Design notes (kept deliberately restful):
//   * The sea is a real perspective plane, not a flat gradient. A small stack of
//     directional swell + chop waves is evaluated in a depth-warped coordinate so
//     the waves stretch and crowd toward the horizon the way real water does. The
//     wave slope reflects the pale near-horizon sky on the up-facing facets and
//     leaves the troughs deep teal, which gives the surface its living shimmer.
//   * The sun is a soft disc on a warm horizon band. Its reflection is a vertical
//     column beneath it that meanders along the actual swell and is chopped into
//     glints by the wave crests — brightest just below the waterline.
//   * Steep near crests catch a whisper of foam; distant water stays glassy.
//   * Optional tiny dark sail triangles perch on the horizon on a sparse jittered
//     grid (`sails` = how many). The canvas pan slides everything gently sideways.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Swell & shimmer" group="Ocean"
// @prop sun_height float default=1.0 min=0.2 max=2.0 label="Sun height" group="Ocean"
// @prop glitter float default=1.0 min=0.0 max=2.0 label="Glitter" group="Ocean"
// @prop sails float default=0.4 min=0.0 max=1.0 label="Distant sails" group="Ocean"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Ocean"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Ocean"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Ocean"

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

// Driver-stable integer/bit-mix value hash (Dave Hoskins) — no `fract(sin)`, so
// the noise stays box-free across Vulkan drivers.
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
    for (var i = 0; i < 4; i = i + 1) {
        v = v + a * noise(p);
        p = p * 2.0;
        a = a * 0.5;
    }
    return v;
}

// Analytic wave field in wave-space. Returns vec3(height, dH/dx, dH/dz): a small
// stack of directional sines, each roughly half the amplitude and ~1.8x the
// frequency of the last (swell → chop). The gradient is exact (cosine terms), so
// we get clean per-facet slopes for lighting without finite differences.
fn waves(p: vec2<f32>, t: f32) -> vec3<f32> {
    var h = 0.0;
    var g = vec2<f32>(0.0);
    var amp = 1.0;
    var freq = 1.0;
    for (var i = 0; i < 5; i = i + 1) {
        // Swell travels mostly toward the eye (+z) so crests read as horizontal
        // lines that crowd toward the horizon; a little sideways lean per octave.
        let a = f32(i) * 2.4 + 0.6;
        let dir = normalize(vec2<f32>(sin(a) * 0.45, 1.0));
        let ph = dot(dir, p) * freq - t * (1.0 + f32(i) * 0.35);
        h = h + amp * sin(ph);
        g = g + amp * cos(ph) * freq * dir;
        amp = amp * 0.5;
        freq = freq * 1.9;
    }
    return vec3<f32>(h, g.x, g.y);
}

// The horizon sits a touch above centre so there's more water than sky.
const HORIZON: f32 = 0.06;

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
    let sun_height = pc.params[0].y;
    let glitter = pc.params[0].z;
    let sails = pc.params[0].w;
    let vignette = pc.params[1].x;
    let vig_radius = pc.params[1].y;
    let vig_softness = pc.params[1].z;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    let y = -uv.y + pan.y * 0.00030;             // gentle vertical pan
    let x = uv.x + pan.x * 0.00030;              // gentle horizontal pan

    // Sun a little above the waterline; its mirror image sits the same distance below.
    let sun_x = 0.0;
    let sun_y = HORIZON + 0.055 * sun_height;
    let sun_col = vec3<f32>(1.0, 0.86, 0.62);

    // --- Sky: pale + warm at the horizon, cooling to blue overhead ---
    var sky = mix(vec3<f32>(0.86, 0.86, 0.82), vec3<f32>(0.30, 0.48, 0.72), smoothstep(HORIZON, 0.75, y));
    // Warm band hugging the waterline (the sun's glow bleeding into the low sky).
    let warm = exp(-max(y - HORIZON, 0.0) * 11.0);
    sky = sky + vec3<f32>(0.24, 0.13, 0.03) * warm * 0.6;
    // A whisper of high cloud, thinning toward the horizon so the band stays clean.
    let cl = fbm(vec2<f32>(x * 1.8 + pan.x * 0.0003 + time * 0.010 * drift, y * 3.2 - 4.0));
    sky = sky + vec3<f32>(0.07, 0.06, 0.05) * smoothstep(0.58, 0.85, cl) * smoothstep(HORIZON + 0.02, 0.45, y);
    let sun_sky = length(vec2<f32>(x - sun_x, y - sun_y));
    sky = sky + sun_col * smoothstep(0.085, 0.0, sun_sky);                        // soft disc
    sky = sky + vec3<f32>(0.6, 0.5, 0.36) * exp(-sun_sky * sun_sky * 8.0) * 0.7;  // halo

    // --- Sea: a receding wave plane, reflecting the sky, with a glitter road ---
    let below = HORIZON - y;                                   // >0 in the sea
    // Perspective depth: large (far) at the horizon, small (near) toward the bottom.
    let pdepth = 0.085 / (below + 0.05);
    let wave_p = vec2<f32>(x * pdepth * 3.2 + pan.x * 0.0004, pdepth * 2.8 + time * 0.03 * drift);
    let w = waves(wave_p, time * 0.9 * drift);
    // Fade wave detail out right at the seam so the horizon line stays crisp.
    let sea_fade = smoothstep(0.0, 0.05, below);
    let slope = w.z * sea_fade;                                // tilt toward/away from the eye
    let steep = length(vec2<f32>(w.y, w.z));                   // crest sharpness

    // Base water: reflected sky near the horizon, deepening to teal below.
    let depth_t = smoothstep(HORIZON, HORIZON - 0.6, y);
    var sea = mix(vec3<f32>(0.18, 0.40, 0.50), vec3<f32>(0.015, 0.07, 0.14), depth_t);
    // Up-facing facets mirror the pale/warm near-horizon sky; down-facing stay deep.
    let facet = clamp(0.5 + 1.3 * slope, 0.0, 1.0);
    let sky_reflect = mix(vec3<f32>(0.30, 0.44, 0.52), vec3<f32>(0.70, 0.76, 0.80), facet);
    sea = mix(sea, sky_reflect, facet * 0.42 * sea_fade);
    // A whisper of foam on the steepest near crests; distant water stays glassy.
    let foam = smoothstep(0.9, 1.6, steep) * smoothstep(0.28, 0.9, below);
    sea = mix(sea, vec3<f32>(0.80, 0.86, 0.88), foam * 0.3);

    // Sun's reflection: a glitter road under the sun that meanders along the real
    // swell and is chopped into sparkling glints by the up-facing wave crests.
    let cx = abs((x - sun_x) + 0.03 * w.x * sea_fade);
    let width = 0.05 + 0.42 * clamp(below, 0.0, 1.0);          // fans out with depth
    let column = exp(-(cx * cx) / (width * width));
    let vert = exp(-max(below, 0.0) * 1.5);                    // spill down the water
    var glint = clamp(0.5 + 1.1 * slope, 0.0, 1.0);          // crests catch the sun
    glint = pow(glint, 3.0);
    let reflection = column * vert * (0.14 + 0.86 * glint) * glitter;
    sea = sea + sun_col * reflection;

    // Join sky and sea across the anti-aliased horizon.
    let e = 0.004 / zoom;
    var col = mix(sea, sky, smoothstep(HORIZON - e, HORIZON + e, y));

    // Distant sails: tiny dark triangles on a sparse jittered grid, sitting on the
    // horizon. Kept small and hazed so they read as far away.
    if (sails > 0.001) {
        let cell = 2.2;
        let gx = (x - pan.x * 0.00010) * cell;                 // faint parallax vs. pan
        let ix = floor(gx);
        for (var k = -1; k <= 1; k = k + 1) {
            let id = ix + f32(k);
            let h = hash(vec2<f32>(id, 7.0));
            if (h < 0.35 * sails) {
                let sx = (id + 0.3 + 0.4 * hash(vec2<f32>(id, 3.0))) / cell;
                let sz = 0.010 + 0.010 * hash(vec2<f32>(id, 5.0));
                let dx = (x - sx);
                let up = y - HORIZON;                           // height above waterline
                // Triangle: base 2*sz at the waterline, apex sz*2.6 tall.
                let halfw = sz * (1.0 - up / (sz * 2.6));
                let inside = step(0.0, up) * step(0.0, sz * 2.6 - up) * step(abs(dx), halfw);
                col = mix(col, vec3<f32>(0.16, 0.18, 0.22), inside * 0.9);
                // A short reflected smudge just under the hull.
                let refl = step(-sz * 1.2, up) * step(up, 0.0) * step(abs(dx), sz * 0.5);
                col = mix(col, col * 0.7, refl * 0.4);
            }
        }
    }

    // Lock-screen ease: dim toward a calm night sea.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.4 + vec3<f32>(0.006, 0.012, 0.02), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

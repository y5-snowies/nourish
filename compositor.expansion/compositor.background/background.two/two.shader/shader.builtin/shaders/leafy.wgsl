// Built-in background: "Leafy Galaxy" — a calm, unobtrusive parallax scene of a
// small cluster of verdant living worlds drifting in deep space. A companion to
// the stock space parallax: same quiet, dark, low-contrast mood, same Push/`@prop`
// contract, so it slots into the built-in shader list and the live preview.
//
// Design notes (kept deliberately restful):
//   * Near-black teal sky with a faint green aurora haze — never competes with
//     the foreground workspace.
//   * Three worlds at different depths, each turned into shadow so their bright
//     limbs stay off-centre and most of the frame reads as calm dark space.
//   * Each globe is shaded as a real sphere (foreshortened continents, a soft
//     day/night terminator, drifting cloud, a thin atmosphere rim) and spins on
//     its axis — the spin rate is driven by the drift-speed knob.
//   * Everything parallaxes with pan by depth: nearer worlds shift less.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Drift & spin" group="Leafy"
// @prop star_density float default=1.0 min=0.0 max=2.0 label="Star density" group="Leafy"
// @prop foliage float default=1.0 min=0.0 max=2.0 label="Foliage & cloud" group="Leafy"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Leafy"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Leafy"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Leafy"

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
// the noise stays box-free across Vulkan drivers (see the stock parallax note).
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

// A slow, sparse starfield in three parallax depths (dim so it never sparkles
// aggressively). `pan` shifts each layer by depth for the parallax cue.
fn stars(uv: vec2<f32>, pan: vec2<f32>, time: f32, density: f32) -> vec3<f32> {
    var col = vec3<f32>(0.0);
    for (var i = 1; i <= 3; i = i + 1) {
        let depth = f32(i) * 0.5;
        let sp = uv * (42.0 / depth) + pan * 0.001 * depth;
        let id = floor(sp);
        let fp = fract(sp) - 0.5;
        let h = hash(id + f32(i) * 11.0);
        if (h > 1.0 - 0.03 * density) {
            let twink = 0.6 + 0.4 * sin(time * 0.8 + h * 40.0);
            let dd = length(fp);
            let tint = mix(vec3<f32>(0.75, 0.95, 0.85), vec3<f32>(0.95, 1.0, 0.9), fract(h * 91.7));
            col = col + tint * smoothstep(0.05, 0.0, dd) * twink / (depth * 2.2);
        }
    }
    return col;
}

// Render the hero planet into `col`. Shaded as a sphere: `n` is the surface
// normal, from which we build foreshortened lon/lat texture coords so the
// continents compress realistically toward the limb.
fn draw_world(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
              rot: f32, foliage: f32) -> vec3<f32> {
    let pp = (uv - center) / radius;
    let r2 = dot(pp, pp);
    let mask = smoothstep(1.0, 1.0 - 0.02 / radius, r2);
    if (mask <= 0.0) { return col; }

    let z = sqrt(max(1.0 - r2, 0.0));
    let n = vec3<f32>(pp, z);
    // Light grazes from the upper-left so the terminator crosses the disc: the
    // corner limb catches the light and the inner face (toward the workspace)
    // falls into a calm shadow.
    let light_dir = normalize(vec3<f32>(-0.5, 0.45, 0.25));
    let day = clamp(dot(n, light_dir), 0.0, 1.0);
    let term = smoothstep(0.0, 0.5, day); // soft day/night terminator

    // Spherical surface coords, foreshortened at the limb; `rot` spins the axis
    // (its rate is the drift-speed knob, applied by the caller).
    let lon = atan2(n.x, n.z) * 0.3183099 + rot;
    let lat = asin(clamp(n.y, -1.0, 1.0)) * 0.6366198;
    let suv = vec2<f32>(lon, lat) * 3.0;

    // Continents vs. oceans, then mossy tone variation across the land.
    let land = fbm(suv * 1.6);
    let landmask = smoothstep(0.48, 0.6, land);
    let ocean = mix(vec3<f32>(0.03, 0.10, 0.13), vec3<f32>(0.05, 0.16, 0.18), fbm(suv * 4.0));
    let grn = fbm(suv * 5.0 + 4.0);
    let veg = mix(vec3<f32>(0.10, 0.24, 0.12), vec3<f32>(0.20, 0.34, 0.16), grn) * (0.7 + 0.6 * foliage);
    var surface = mix(ocean, veg, landmask);

    // Drifting cloud veil — brighter over land, thinning at the poles.
    let cl = fbm(suv * 2.2 + vec2<f32>(rot * 0.6, -rot * 0.25));
    let clouds = smoothstep(0.55, 0.85, cl) * (0.35 + 0.4 * foliage) * (1.0 - abs(n.y) * 0.4);
    surface = mix(surface, vec3<f32>(0.72, 0.80, 0.74), clouds);

    // Night side keeps a faint self-lit teal so the dark limb never reads as a hole.
    let night = vec3<f32>(0.012, 0.03, 0.028);
    var body = mix(night, surface, term);

    // Thin atmosphere: a green rim that only glows on the lit edge (Rayleigh-ish).
    let rim = pow(1.0 - z, 3.0);
    let atmo = vec3<f32>(0.20, 0.42, 0.26) * rim * smoothstep(-0.1, 0.7, day) * 0.9;
    body = body + atmo;

    return mix(col, body, mask);
}

// A small, plain moon (pale grey rock) for a quiet sense of depth. Same sphere
// shading, minus the atmosphere and living colour; `rot` spins its face.
fn draw_moon(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32, rot: f32) -> vec3<f32> {
    let pp = (uv - center) / radius;
    let r2 = dot(pp, pp);
    let mask = smoothstep(1.0, 1.0 - 0.03 / radius, r2);
    if (mask <= 0.0) { return col; }
    let z = sqrt(max(1.0 - r2, 0.0));
    let n = vec3<f32>(pp, z);
    let day = clamp(dot(n, normalize(vec3<f32>(-0.5, 0.45, 0.25))), 0.0, 1.0);
    let tone = 0.11 + 0.14 * fbm(vec2<f32>(atan2(n.x, n.z) * 0.3183099 + rot, n.y) * 6.0);
    // Keep a soft ambient floor so the moon reads as a gentle disc, not a stark
    // crescent.
    let body = vec3<f32>(tone, tone * 1.02, tone * 1.05) * (0.4 + 0.6 * smoothstep(-0.2, 0.7, day));
    return mix(col, body + vec3<f32>(0.008, 0.01, 0.013), mask);
}

// A banded gas giant: horizontal latitude bands with turbulent swirls, a soft
// terminator and a thin rim. `ca`/`cb` are the two band tones.
fn draw_banded(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
               rot: f32, ca: vec3<f32>, cb: vec3<f32>) -> vec3<f32> {
    let pp = (uv - center) / radius;
    let r2 = dot(pp, pp);
    let mask = smoothstep(1.0, 1.0 - 0.02 / radius, r2);
    if (mask <= 0.0) { return col; }
    let z = sqrt(max(1.0 - r2, 0.0));
    let n = vec3<f32>(pp, z);
    let day = clamp(dot(n, normalize(vec3<f32>(-0.5, 0.45, 0.25))), 0.0, 1.0);
    let term = smoothstep(0.0, 0.5, day);
    let lat = asin(clamp(n.y, -1.0, 1.0));
    let lon = atan2(n.x, n.z) * 0.3183099 + rot;
    let turb = fbm(vec2<f32>(lon * 2.5, lat * 3.5)) * 0.6;
    let band = sin(lat * 9.0 + turb * 4.0) * 0.5 + 0.5;
    var surf = mix(ca, cb, band);
    surf = surf * (0.85 + 0.3 * fbm(vec2<f32>(lon * 3.0 + rot, lat * 6.0)));
    var body = mix(surf * 0.12, surf, term);
    let rim = pow(1.0 - z, 3.0);
    body = body + cb * rim * smoothstep(-0.1, 0.7, day) * 0.5;
    return mix(col, body, mask);
}

// A small, self-luminous star: a soft additive corona plus a granular core that
// gently pulses. `core`/`edge` set the disc and halo colour.
fn draw_star(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
             time: f32, core: vec3<f32>, edge: vec3<f32>) -> vec3<f32> {
    let pp = (uv - center) / radius;
    let d = length(pp);
    let glow = exp(-d * d * 2.2) * 0.35 + smoothstep(1.6, 0.0, d) * 0.06;
    var body = col + edge * glow;
    let disc = smoothstep(1.0, 0.8, d);
    let gran = 0.85 + 0.3 * fbm(pp * 6.0 + time * 0.05);
    let pulse = 0.92 + 0.08 * sin(time * 0.8);
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
    let foliage = pc.params[0].z;
    let vignette = pc.params[0].w;
    let vig_radius = pc.params[1].x;
    let vig_softness = pc.params[1].y;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    // Pan convention: on-screen content tracks the camera as -pan_in on both axes.
    let pan = vec2<f32>(pan_in.x, pan_in.y);

    // Deep teal-to-navy sky, darkest at the top.
    var col = mix(vec3<f32>(0.015, 0.045, 0.05), vec3<f32>(0.01, 0.02, 0.045), frag.y / res.y);

    // Faint green aurora haze, slow drift; a whisper of colour, never a wash.
    let haze_uv = uv * 1.3 + pan * 0.00018 + flow * 0.0003 + vec2<f32>(time * 0.008, time * 0.004) * drift;
    let h1 = fbm(haze_uv);
    col = col + mix(vec3<f32>(0.03, 0.14, 0.10), vec3<f32>(0.02, 0.08, 0.12), h1) * pow(h1, 2.0) * 0.35 * (0.6 + 0.4 * foliage);

    col = col + stars(uv, pan, time, star_density);

    // Drift-driven axial spin phase (higher knob → faster). Farther worlds turn
    // a touch faster in apparent terms; each parallaxes by its depth.
    let spin = time * drift * 0.05;

    // A varied trio at different depths: the lush hero world, a cool banded gas
    // giant, and a small pale star. The hero (nearest) shifts least.
    col = draw_world(col, uv, vec2<f32>(-0.52, -0.36) - pan * 0.00030, 0.34, spin, foliage);
    col = draw_banded(col, uv, vec2<f32>(0.62, 0.24) - pan * 0.00060, 0.17, spin * 1.2,
                      vec3<f32>(0.11, 0.22, 0.24), vec3<f32>(0.05, 0.12, 0.15));
    col = draw_star(col, uv, vec2<f32>(0.1, 0.46) - pan * 0.00095, 0.045, time,
                    vec3<f32>(0.85, 0.98, 0.82), vec3<f32>(0.3, 0.5, 0.32));

    // Lock-screen ease: settle the scene into a darker, stiller night.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.45 + vec3<f32>(0.004, 0.012, 0.012), l);

    // Optional edge vignette in zoom-independent screen space.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

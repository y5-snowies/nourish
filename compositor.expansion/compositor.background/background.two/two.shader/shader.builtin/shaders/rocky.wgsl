// Built-in background: "Rocky Galaxy" — a calm, unobtrusive parallax scene of a
// small cluster of barren, cratered worlds drifting in deep space. A companion to
// the stock space parallax: same quiet, dark, low-contrast mood, same Push/`@prop`
// contract.
//
// Design notes (kept deliberately restful):
//   * Near-black sky with a faint warm dust haze — a whisper of rust colour.
//   * Three worlds at different depths, side-lit so most of each disc is a soft
//     shadow and the bright limbs sit away from the workspace.
//   * The surfaces are genuinely cratered (a cellular/Worley field carves bowls
//     and rims) over broad brown highland/basin albedo, foreshortened onto the
//     sphere. Each world spins on its axis, driven by the drift-speed knob.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Drift & spin" group="Rocky"
// @prop star_density float default=1.0 min=0.0 max=2.0 label="Star density" group="Rocky"
// @prop ruggedness float default=1.0 min=0.0 max=2.0 label="Ruggedness" group="Rocky"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Rocky"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Rocky"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Rocky"

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

// Driver-stable integer/bit-mix hashes (Dave Hoskins) — no `fract(sin)`.
fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}
fn hash2(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract(vec2<f32>((p3.x + p3.y) * p3.z, (p3.x + p3.z) * p3.y));
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

// Worley over one cell neighbourhood: returns the distance to the nearest
// jittered cell point (`.x`), that cell's hash (`.y`), and the outward direction
// from the crater centre to this point (`.zw`) so craters can be lit directionally.
fn worley(p: vec2<f32>) -> vec4<f32> {
    let ip = floor(p);
    let fp = fract(p);
    var f1 = 8.0;
    var id = 0.0;
    var to_center = vec2<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let g = vec2<f32>(f32(x), f32(y));
            let o = hash2(ip + g);
            let rvec = g + o - fp;            // this point → crater centre
            let d = length(rvec);
            if (d < f1) { f1 = d; id = hash(ip + g); to_center = rvec; }
        }
    }
    return vec4<f32>(f1, id, to_center);
}

// A crater shading term at surface coord `s`, lit from `lgt` (surface space).
// Real craters read as a bright rim crescent on the sun side plus a shadowed
// bowl interior — not uniform rings. Returns a signed shade for the albedo.
fn craters(s: vec2<f32>, scale: f32, lgt: vec2<f32>, rug: f32) -> f32 {
    let w = worley(s * scale);
    let f1 = w.x;
    let radius = 0.30 + 0.18 * w.y;              // per-crater size (varied)
    if (f1 > radius) { return 0.0; }
    let outward = normalize(-w.zw + vec2<f32>(1e-4, 0.0)); // centre → point
    let sun = dot(outward, lgt);                 // +1 on the sun-facing side
    // Bowl interior shadow, deepest toward the sun-facing inner wall.
    let bowl = smoothstep(radius, 0.0, f1) * (0.55 + 0.45 * sun) * 0.34;
    // Thin raised rim, catching light only on the sun side.
    let rim = smoothstep(radius, radius - 0.06, f1)
            * smoothstep(radius - 0.18, radius - 0.06, f1)
            * max(sun, 0.0) * 0.5;
    return (rim - bowl) * rug;
}

fn stars(uv: vec2<f32>, pan: vec2<f32>, time: f32, density: f32) -> vec3<f32> {
    var col = vec3<f32>(0.0);
    for (var i = 1; i <= 3; i = i + 1) {
        let depth = f32(i) * 0.5;
        let sp = uv * (42.0 / depth) + pan * 0.001 * depth;
        let id = floor(sp);
        let fp = fract(sp) - 0.5;
        let h = hash(id + f32(i) * 13.0);
        if (h > 1.0 - 0.03 * density) {
            let twink = 0.6 + 0.4 * sin(time * 0.7 + h * 40.0);
            let dd = length(fp);
            let tint = mix(vec3<f32>(0.8, 0.85, 0.95), vec3<f32>(0.95, 0.9, 0.82), fract(h * 71.3));
            col = col + tint * smoothstep(0.05, 0.0, dd) * twink / (depth * 2.4);
        }
    }
    return col;
}

// The hero planet: a sphere with foreshortened cratered rock and a thin dusty
// terminator glow.
fn draw_rock(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
             rot: f32, rug: f32) -> vec3<f32> {
    let pp = (uv - center) / radius;
    let r2 = dot(pp, pp);
    let mask = smoothstep(1.0, 1.0 - 0.02 / radius, r2);
    if (mask <= 0.0) { return col; }

    let z = sqrt(max(1.0 - r2, 0.0));
    let n = vec3<f32>(pp, z);
    let light_dir = normalize(vec3<f32>(0.55, 0.4, 0.22));  // grazes from the right
    let day = clamp(dot(n, light_dir), 0.0, 1.0);
    let term = smoothstep(-0.05, 0.55, day);

    // Foreshortened surface coords; `rot` spins the axis (drift-speed knob).
    let lon = atan2(n.x, n.z) * 0.3183099 + rot;
    let lat = asin(clamp(n.y, -1.0, 1.0)) * 0.6366198;
    let s = vec2<f32>(lon, lat) * 2.6;

    // Broad albedo: dark basins vs. paler highlands, warm rusty brown.
    let maria = fbm(s * 1.4);
    let hi = vec3<f32>(0.36, 0.24, 0.145);
    let lo = vec3<f32>(0.16, 0.095, 0.055);
    var alb = mix(lo, hi, smoothstep(0.35, 0.65, maria));

    // Two crater octaves (big basins + smaller pocks), lit from a fixed sun in
    // surface space so the relief reads consistently across the disc.
    let sun2d = normalize(vec2<f32>(0.7, 0.55));
    let c = craters(s, 2.1, sun2d, rug) + craters(s + 11.0, 4.6, sun2d, rug) * 0.55;
    alb = clamp(alb + vec3<f32>(c), vec3<f32>(0.0), vec3<f32>(1.0));
    // Fine grain so the terminator isn't glassy.
    alb = alb * (0.9 + 0.2 * fbm(s * 14.0));

    // Night side keeps a faint ember of reflected starlight, not pure black.
    let night = vec3<f32>(0.022, 0.015, 0.012);
    var body = mix(night, alb, term);

    // Thin, dusty rim that only catches the light — a warm brown haze, no
    // lush atmosphere here.
    let rim = pow(1.0 - z, 3.5);
    body = body + vec3<f32>(0.34, 0.20, 0.10) * rim * smoothstep(-0.1, 0.7, day) * 0.5;

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
    let day = clamp(dot(n, normalize(vec3<f32>(0.55, 0.4, 0.22))), 0.0, 1.0);
    let term = smoothstep(-0.05, 0.55, day);
    let lat = asin(clamp(n.y, -1.0, 1.0));
    let lon = atan2(n.x, n.z) * 0.3183099 + rot;
    let turb = fbm(vec2<f32>(lon * 2.5, lat * 3.5)) * 0.6;
    let band = sin(lat * 8.0 + turb * 4.0) * 0.5 + 0.5;
    var surf = mix(ca, cb, band);
    surf = surf * (0.85 + 0.3 * fbm(vec2<f32>(lon * 3.0 + rot, lat * 6.0)));
    var body = mix(surf * 0.12, surf, term);
    let rim = pow(1.0 - z, 3.0);
    body = body + cb * rim * smoothstep(-0.1, 0.7, day) * 0.5;
    return mix(col, body, mask);
}

// A ringed world: a banded planet wrapped by a tilted, textured ring. The ring's
// far arc is occluded by the disc; its near arc is drawn over it.
fn draw_ringed(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
               rot: f32, ca: vec3<f32>, cb: vec3<f32>, ringc: vec3<f32>) -> vec3<f32> {
    var c = col;
    let pr = (uv - center) / radius;
    let d2 = dot(pr, pr);
    let pmask = smoothstep(1.0, 1.0 - 0.02 / radius, d2);

    // Ring in a tilted plane — squash y so we see it near edge-on.
    let rp = vec2<f32>(pr.x, pr.y / 0.32);
    let rr = length(rp);
    let tex = 0.55 + 0.45 * fbm(vec2<f32>(rr * 9.0, 0.0));
    let inner = smoothstep(1.36, 1.42, rr);
    let outer = 1.0 - smoothstep(2.02, 2.12, rr);
    let cassini = 1.0 - 0.7 * (smoothstep(1.70, 1.73, rr) * (1.0 - smoothstep(1.80, 1.83, rr)));
    let ralpha = inner * outer * cassini * tex;
    let near = step(pr.y, 0.0);
    // Far arc: behind the planet, so only where outside the disc.
    c = mix(c, ringc, ralpha * (1.0 - near) * (1.0 - pmask) * 0.9);

    // Banded planet body.
    let z = sqrt(max(1.0 - d2, 0.0));
    let n = vec3<f32>(pr, z);
    let day = clamp(dot(n, normalize(vec3<f32>(0.55, 0.4, 0.22))), 0.0, 1.0);
    let term = smoothstep(-0.05, 0.55, day);
    let lat = asin(clamp(n.y, -1.0, 1.0));
    let band = sin(lat * 7.0 + fbm(vec2<f32>(n.x * 3.0, lat * 4.0)) * 3.0) * 0.5 + 0.5;
    var body = mix(mix(ca, cb, band) * 0.12, mix(ca, cb, band), term);
    body = body + cb * pow(1.0 - z, 3.0) * smoothstep(-0.1, 0.7, day) * 0.4;
    c = mix(c, body, pmask);

    // Near arc: drawn over the planet.
    c = mix(c, ringc, ralpha * near * 0.9);
    return c;
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
    let ruggedness = pc.params[0].z;
    let vignette = pc.params[0].w;
    let vig_radius = pc.params[1].x;
    let vig_softness = pc.params[1].y;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    // Dark brown-charcoal sky with the faintest warm lift toward the bottom.
    var col = mix(vec3<f32>(0.035, 0.026, 0.022), vec3<f32>(0.018, 0.013, 0.012), frag.y / res.y);

    // Faint warm rust dust haze, drifting slowly.
    let haze_uv = uv * 1.2 + pan * 0.00016 + flow * 0.0003 + vec2<f32>(time * 0.007, time * 0.003) * drift;
    let h1 = fbm(haze_uv);
    col = col + mix(vec3<f32>(0.13, 0.06, 0.03), vec3<f32>(0.06, 0.045, 0.04), h1) * pow(h1, 2.2) * 0.32;

    col = col + stars(uv, pan, time, star_density);

    // Drift-driven axial spin; farther worlds turn a touch faster and parallax more.
    let spin = time * drift * 0.05;

    // A varied trio: the cratered hero, a dusty banded gas giant, and a small
    // ringed world.
    col = draw_rock(col, uv, vec2<f32>(0.5, -0.34) - pan * 0.00030, 0.36, spin, ruggedness);
    col = draw_banded(col, uv, vec2<f32>(-0.6, 0.22) - pan * 0.00060, 0.17, spin * 1.2,
                      vec3<f32>(0.30, 0.21, 0.13), vec3<f32>(0.17, 0.11, 0.07));
    col = draw_ringed(col, uv, vec2<f32>(0.12, 0.4) - pan * 0.00095, 0.075, spin * 1.5,
                      vec3<f32>(0.26, 0.20, 0.14), vec3<f32>(0.15, 0.11, 0.08), vec3<f32>(0.26, 0.21, 0.16));

    // Lock-screen ease: settle into a darker, stiller night.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.45 + vec3<f32>(0.01, 0.008, 0.008), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

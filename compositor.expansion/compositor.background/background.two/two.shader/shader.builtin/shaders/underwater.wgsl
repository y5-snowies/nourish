// Built-in background: "Underwater" — a calm, unobtrusive descent into a deep
// blue-green sea. A companion to the other built-in scenes: same quiet, dark,
// low-contrast mood, same Push/`@prop` contract, so it slots into the built-in
// shader list and the live preview.
//
// Design notes (kept deliberately restful, but properly layered so it reads as a
// real body of water rather than a flat wash):
//   * Depth-absorption column — a lighter teal just under the surface (top)
//     sinking to a near-black navy in the deep (bottom), on a perceptual curve so
//     the falloff mimics how water swallows light. Domain-warped fbm gives the
//     body a slow, volumetric swell instead of a linear gradient.
//   * Caustics — the signature underwater tell. An animated Voronoi-edge network
//     (thin, moving bright filaments) shimmers through the upper water where the
//     refracted surface light concentrates, and the same field flickers along the
//     god rays so the shafts sparkle rather than sit static.
//   * God rays — soft, tilted parallel shafts of surface light that rake in from
//     above, gently warped so they curve and breathe, brightest near the surface
//     and gone in the deep.
//   * Bubbles rising in parallax layers — translucent spheres with a Fresnel rim
//     (bright at the grazing edge), a top-lit specular highlight and a soft core,
//     analytically anti-aliased, wobbling side to side as they climb. The canvas
//     pan carries them for a parallax cue.
//   * Marine snow — depth-of-field motes drifting down through the mid-water: near
//     layers are large, soft bokeh discs, far layers tiny and dim, so the volume
//     never feels empty or flat.
//   * A soft filmic highlight shoulder tames the caustic/ray sparkle into a glow
//     instead of hard clipping, and the deep desaturates slightly for mood.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Current & rise" group="Underwater"
// @prop bubble_density float default=1.0 min=0.0 max=2.0 label="Bubbles" group="Underwater"
// @prop light_shafts float default=1.0 min=0.0 max=2.0 label="Light shafts" group="Underwater"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Underwater"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Underwater"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Underwater"
// @prop clarity float default=1.0 min=0.0 max=2.0 label="Water clarity" group="Underwater"
// @prop caustics float default=1.0 min=0.0 max=2.0 label="Caustics" group="Underwater"

struct Push {
    res_zoom_time: vec4<f32>,        // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,             // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,           // x = lock_amount, y = alpha
    params: array<vec4<f32>, 4>,     // shader-authored @prop values (16 floats)
};
var<immediate> pc: Push;

// ── Reserved: texture / animated sprite-sheet slot (not yet wired) ────────────
// The renderer currently drives background shaders with push constants only —
// there is no bound texture or sampler in the dispatch seam. This block stakes
// out the contract so a sprite-sheet atlas can be dropped in without reworking
// the shader once the engine gains a texture descriptor for pixel programs.
//
// `params[3]` is reserved as the sprite-sheet control vec4 (zero-filled today):
//   params[3].x = atlas columns        params[3].z = playback fps
//   params[3].y = atlas rows           params[3].w = frame count (0 = cols*rows)
//
// When a texture arrives, bind it here and switch on the helper below:
//   @group(0) @binding(0) var atlas_tex: texture_2d<f32>;
//   @group(0) @binding(1) var atlas_smp: sampler;
//
// Sub-rect UV for the current animation frame of a cols×rows sheet. `cell` is the
// 0..1 coord within one sprite (e.g. a bubble's local quad remapped to 0..1).
fn sprite_frame_uv(cell: vec2<f32>, cols: f32, rows: f32, fps: f32, count: f32, time: f32) -> vec2<f32> {
    let total = select(cols * rows, count, count > 0.5);
    let frame = floor(time * max(fps, 0.0)) % max(total, 1.0);
    let fx = frame % cols;
    let fy = floor(frame / cols);
    return (vec2<f32>(fx, fy) + clamp(cell, vec2<f32>(0.0), vec2<f32>(1.0))) / vec2<f32>(cols, rows);
}
// ─────────────────────────────────────────────────────────────────────────────

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

// Driver-stable integer/bit-mix value hashes (Dave Hoskins) — no `fract(sin)`, so
// the noise stays box-free across Vulkan drivers (see the stock parallax note).
fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}
fn hash2(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract(vec2<f32>(p3.x + p3.y, p3.y + p3.z) * vec2<f32>(p3.z, p3.x));
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
// Rotated-octave fbm — the per-octave twist keeps the swell from lining up with
// the grid axes, so the body reads as organic turbulence, not a tiled wash.
fn fbm(p_in: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var p = p_in;
    let m = mat2x2<f32>(0.80, 0.60, -0.60, 0.80);
    for (var i = 0; i < 5; i = i + 1) {
        v = v + a * noise(p);
        p = m * p * 2.0;
        a = a * 0.5;
    }
    return v;
}

// One octave of an animated Voronoi *edge* field: bright, thin filaments along the
// cell borders (F2-F1), each feature point orbiting slowly in time. This is what
// gives caustics their moving spider-web of concentrated light.
fn caustic_edge(p: vec2<f32>, time: f32) -> f32 {
    let n = floor(p);
    let f = fract(p);
    var f1 = 8.0;
    var f2 = 8.0;
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            let g = vec2<f32>(f32(i), f32(j));
            let o = hash2(n + g);
            let pt = g + 0.5 + 0.42 * sin(time + 6.2831 * o);
            let r = pt - f;
            let d = dot(r, r);
            if (d < f1) { f2 = f1; f1 = d; }
            else if (d < f2) { f2 = d; }
        }
    }
    let edge = sqrt(f2) - sqrt(f1);
    return 1.0 - smoothstep(0.0, 0.09, edge);      // bright right at the borders
}
// Two detuned octaves multiplied and squared → sparse, crisp caustic filaments.
fn caustics(p: vec2<f32>, time: f32) -> f32 {
    let a = caustic_edge(p, time);
    let b = caustic_edge(p * 1.8 + vec2<f32>(5.2, 1.3), time * 1.35 + 2.0);
    let c = a * (0.55 + 0.6 * b);
    return c * c;
}

// Soft, tilted parallel shafts of surface light raking in from above. `s` is
// zoom-independent screen space so the light stays anchored to the surface, not
// the world. A slow lateral warp curves the shafts so they breathe; brightest
// just under the surface (top, s.y < 0), gone in the deep.
fn god_rays(s: vec2<f32>, time: f32, drift: f32) -> f32 {
    var q = s;
    q.x = q.x + 0.06 * sin(s.y * 2.0 + time * 0.15 * drift);
    let ang = 0.20;                                 // lean in from the upper-left
    let x = cos(ang) * q.x - sin(ang) * q.y;
    let t = time * 0.04 * drift;
    // A few detuned sine lobes make irregular shafts; drifting noise breaks up
    // their edges so they never read as a fixed grating.
    var sh = pow(0.5 + 0.5 * sin(x * 6.0 + t), 5.0);
    sh = sh + 0.55 * pow(0.5 + 0.5 * sin(x * 3.3 - t * 0.7 + 1.7), 7.0);
    sh = sh + 0.35 * pow(0.5 + 0.5 * sin(x * 10.0 + t * 1.3), 4.0);
    sh = sh * (0.55 + 0.75 * noise(vec2<f32>(x * 2.0, t)));
    let depth = smoothstep(0.6, -0.7, s.y);
    return max(sh, 0.0) * depth;
}

// One parallax layer of rising bubbles. Cells scroll upward over time (so the
// bubbles climb); `pan` carries them for the parallax cue. Each live cell holds a
// translucent sphere: a Fresnel rim that flares at the grazing edge, a soft
// transmitting core and a top-lit specular glint, analytically anti-aliased via
// `ry` (= res.y * zoom, the cell-space size of one pixel) so edges stay crisp at
// any zoom without derivatives in branchy control flow.
fn bubble_layer(col: vec3<f32>, uv: vec2<f32>, pan: vec2<f32>, time: f32,
                i: i32, drift: f32, density: f32, clarity: f32, ry: f32) -> vec3<f32> {
    let depth = 1.0 + f32(i) * 0.85;
    let scale = 6.5 / depth;
    let rise = time * (0.85 * drift) / depth;                // upward scroll (cell space)
    let sp = vec2<f32>(uv.x * scale + pan.x * 0.0012 * depth,
                       uv.y * scale + rise + pan.y * 0.0012 * depth);
    let id = floor(sp);
    let f = fract(sp) - 0.5;
    let h = hash(id + f32(i) * 23.0);
    if (h > 1.0 - 0.05 * density) {
        // Side-to-side sway as it climbs; radius varies a little per bubble.
        let wob = sin(time * 0.7 * drift + h * 30.0) * 0.16;
        let p = f - vec2<f32>(wob, 0.0);
        let rr = 0.12 + 0.15 * fract(h * 57.0);
        let d = length(p);
        let aa = max(scale / max(ry, 1.0), 0.0015) * 1.5;    // ~1.5px feather
        let disc = smoothstep(rr + aa, rr - aa, d);
        let e = clamp(d / rr, 0.0, 1.0);                     // 0 core → 1 rim
        // Fresnel rim: light grazes the meniscus, so brightness climbs to the edge.
        let rim = disc * pow(e, 3.0);
        // Soft transmitting core, faintly brighter toward the middle.
        let fill = disc * (0.05 + 0.05 * (1.0 - e));
        // Top-lit specular glint where the surface light would catch the bubble.
        let hl = smoothstep(0.06, 0.0, length(p - vec2<f32>(-0.30, -0.30) * rr));
        let tint = vec3<f32>(0.52, 0.83, 0.90) * (0.6 + 0.4 * clarity);
        let bub = (tint * fill + tint * rim * 0.7
                   + vec3<f32>(0.85, 0.95, 1.0) * hl * 0.6) / depth;
        return col + bub;
    }
    return col;
}

// One parallax layer of marine snow: sparse motes drifting down. Near layers
// (small `i`) are larger, softer bokeh discs; far layers are tiny and dim. A jitter
// per cell scatters them off the grid and a gentle sway rides the current.
fn snow_layer(col: vec3<f32>, uv: vec2<f32>, pan: vec2<f32>, time: f32,
              i: i32, drift: f32, clarity: f32) -> vec3<f32> {
    let depth = 1.0 + f32(i) * 1.3;
    let scale = 16.0 + f32(i) * 9.0;
    let fall = time * 0.10 * drift / depth;
    let sway = sin(time * 0.2 + f32(i) * 1.7) * 0.3;
    let sp = vec2<f32>(uv.x * scale + pan.x * 0.0012 * depth + sway,
                       uv.y * scale + fall + pan.y * 0.0012 * depth);
    let id = floor(sp);
    let f = fract(sp) - 0.5;
    let h = hash(id + f32(i) * 51.0);
    if (h > 0.94) {
        let jit = (vec2<f32>(hash(id + 1.3), hash(id + 2.7)) - 0.5) * 0.5;
        let d = length(f - jit);
        let rr = (0.06 + 0.14 * fract(h * 91.0)) / depth;    // near = bigger bokeh
        let soft = smoothstep(rr, 0.0, d);
        let glow = pow(soft, 1.5) * (0.11 + 0.09 * fract(h * 13.0));
        return col + vec3<f32>(0.34, 0.50, 0.52) * glow * clarity / depth;
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
    let flow = pc.pan_flow.zw;
    let lock_amount = pc.lock_alpha.x;
    let alpha = pc.lock_alpha.y;

    let drift = pc.params[0].x;
    let bubble_density = pc.params[0].y;
    let light_shafts = pc.params[0].z;
    let vignette = pc.params[0].w;
    let vig_radius = pc.params[1].x;
    let vig_softness = pc.params[1].y;
    let clarity = pc.params[1].z;
    let caustic_amt = pc.params[1].w;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let ry = res.y * zoom;                          // cell-space size of one pixel

    // Depth-absorption column: lighter teal just under the surface (top) sinking to
    // a near-black navy in the deep (bottom), on a perceptual curve so the light
    // dies off the way water actually swallows it.
    let t = frag.y / res.y;
    var col = mix(vec3<f32>(0.06, 0.24, 0.28), vec3<f32>(0.006, 0.030, 0.060), pow(t, 0.85));
    col = col * (0.7 + 0.3 * clarity);

    // A large, slow swell of body colour so the water isn't a flat wash. The fbm is
    // fed back into itself (domain warp) so the turbulence folds and drifts gently
    // with the current and the canvas pan.
    let base = uv * 1.1 + pan * 0.0002 + flow * 0.0003 + vec2<f32>(time * 0.010 * drift, time * 0.004 * drift);
    let w1 = fbm(base);
    let swell = fbm(base + vec2<f32>(w1, w1 * 0.7));
    col = col + mix(vec3<f32>(0.012, 0.050, 0.060), vec3<f32>(0.0, 0.020, 0.040), swell) * pow(swell, 1.5) * 0.6;

    // Caustics: an animated Voronoi-edge network shimmering through the upper water
    // where refracted surface light concentrates. Strongest near the surface,
    // faded out with depth, warped a touch by the swell for a liquid feel.
    let cp = uv * 2.6 + vec2<f32>(w1 * 0.3, 0.0) + pan * 0.0006 + vec2<f32>(0.0, -time * 0.02 * drift);
    let caus = caustics(cp, time * 0.5 * drift);
    let surf = smoothstep(0.15, -0.85, screen_uv.y);
    col = col + vec3<f32>(0.10, 0.28, 0.26) * caus * surf * (0.4 + 0.6 * clarity) * caustic_amt;

    // Faint underside-of-surface glow, brightest at the very top.
    let top = smoothstep(-0.35, -0.95, screen_uv.y);
    col = col + vec3<f32>(0.06, 0.16, 0.17) * top * (0.5 + 0.5 * clarity);

    // Marine snow drifting down through the mid-water, far → near.
    col = snow_layer(col, uv, pan, time, 2, drift, clarity);
    col = snow_layer(col, uv, pan, time, 1, drift, clarity);
    col = snow_layer(col, uv, pan, time, 0, drift, clarity);

    // God rays raking in from the surface, made to sparkle by the caustic field so
    // the shafts flicker with the water instead of sitting static.
    let gr = god_rays(screen_uv, time, drift);
    let grc = gr * (0.6 + 0.8 * caustics(screen_uv * 3.0 + vec2<f32>(3.0, 0.0), time * 0.7 * drift) * caustic_amt);
    col = col + vec3<f32>(0.18, 0.34, 0.34) * grc * 0.5 * light_shafts;

    // Bubbles, far → near (near layer drawn last, on top).
    col = bubble_layer(col, uv, pan, time, 2, drift, bubble_density, clarity, ry);
    col = bubble_layer(col, uv, pan, time, 1, drift, bubble_density, clarity, ry);
    col = bubble_layer(col, uv, pan, time, 0, drift, bubble_density, clarity, ry);

    // Lock-screen ease: sink the scene deeper and stiller, dimming the light.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.45 + vec3<f32>(0.004, 0.012, 0.016), l);

    // Soft filmic highlight shoulder: keeps the caustic/ray sparkle from clipping
    // to hard white, rolling it into a glow. The deep desaturates slightly for mood.
    col = col / (1.0 + col * 0.55) * 1.28;
    let lum = dot(col, vec3<f32>(0.30, 0.59, 0.11));
    col = mix(col, vec3<f32>(lum), smoothstep(0.45, 1.0, t) * 0.22);

    // Optional edge vignette in zoom-independent screen space.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

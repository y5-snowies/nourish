// Built-in background: "Firefly Meadow" — a calm night meadow. A cozy companion
// to the other built-in scenes: same quiet, dark, low-contrast mood, same
// Push/`@prop` contract, so it slots into the built-in shader list and the live
// preview. Shares the star-speck logic of the space parallax, warmed into
// drifting fireflies over a silhouetted field of grass.
//
// Design notes (kept deliberately restful):
//   * A dark green-blue vertical gradient — a deep teal night sky at the top
//     sinking to a warmer, dark meadow green near the ground, with a faint warm
//     ground mist so the base never reads as a flat band.
//   * Layered grass-blade silhouettes at the bottom: tapered, leaning blades in
//     three depth layers (far/lighter → near/near-black), each swaying very
//     gently so the field breathes without ever being busy.
//   * Fireflies: small warm dots that softly blink (a sharp breathing pulse) and
//     drift — wandering side to side while slowly rising, parallaxing with the
//     canvas pan by depth. Some sit behind the grass and some in front.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Drift & sway" group="Firefly Meadow"
// @prop firefly_density float default=1.0 min=0.0 max=2.0 label="Firefly count" group="Firefly Meadow"
// @prop glow float default=1.0 min=0.0 max=2.0 label="Firefly glow" group="Firefly Meadow"
// @prop pulse float default=1.0 min=0.0 max=3.0 label="Blink speed" group="Firefly Meadow"
// @prop grass_height float default=1.0 min=0.0 max=2.0 label="Grass height" group="Firefly Meadow"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Firefly Meadow"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Firefly Meadow"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Firefly Meadow"

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
// out the contract so a sprite-sheet atlas (e.g. a real glowing-firefly sprite)
// can be dropped in without reworking the shader once the engine gains a texture
// descriptor for pixel programs.
//
// `params[3]` is reserved as the sprite-sheet control vec4 (zero-filled today):
//   params[3].x = atlas columns        params[3].z = playback fps
//   params[3].y = atlas rows           params[3].w = frame count (0 = cols*rows)
//
// When a texture arrives, bind it here and switch the fireflies onto the helper:
//   @group(0) @binding(0) var atlas_tex: texture_2d<f32>;
//   @group(0) @binding(1) var atlas_smp: sampler;
//
// Sub-rect UV for the current animation frame of a cols×rows sheet. `cell` is the
// 0..1 coord within one sprite (a firefly's local quad remapped to 0..1).
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
    for (var i = 0; i < 4; i = i + 1) {
        v = v + a * noise(p);
        p = p * 2.0;
        a = a * 0.5;
    }
    return v;
}

// Distance from `p` to segment a→b, plus the 0..1 position along it (`.y`), so a
// blade can taper from base to tip.
fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let pa = p - a;
    let ba = b - a;
    let u = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return vec2<f32>(length(pa - ba * u), u);
}

// Coverage of one grass layer at screen point `px`, returned as
// `vec2(coverage, tip_fraction)`: `.x` is the silhouette weight in [0,1] and `.y`
// is how far up the winning blade the point sits (0 = base, 1 = tip). The caller
// uses `.y` to fade blade tips into the night air, so the field ends in a soft
// feathered edge instead of hard spikes. Blades rise from `ground` (screen-space
// y, +y = down) to a per-blade length, leaning and gently swaying; each is a
// segment that tapers to a point at the tip. Neighbour cells are checked so
// leaning blades from adjacent cells overlap.
fn grass_cover(px: vec2<f32>, ground: f32, height: f32, count: f32, sway: f32, seed: f32) -> vec2<f32> {
    let cw = 1.0 / count;               // blade cell width, in screen-x units
    let s = px.x * count;
    let cell = floor(s);
    var cover = 0.0;
    var tipf = 0.0;
    for (var k = -1; k <= 1; k = k + 1) {
        let c = cell + f32(k);
        let h = 0.45 + 0.55 * hash(vec2<f32>(c, seed));
        let lean = (hash(vec2<f32>(c, seed + 17.3)) - 0.5) * 1.4;
        let phase = hash(vec2<f32>(c, seed + 5.1)) * 6.2831853;
        let bx = (c + 0.5) * cw;                       // blade base x
        let tip = vec2<f32>(bx + (lean + sway * sin(phase)) * cw * 3.0, ground - height * h);
        let base = vec2<f32>(bx, ground);
        let r = seg_dist(px, base, tip);
        // Widen the AA band toward the tip so blade edges soften as they thin,
        // and keep a little width at the very tip so it rounds off instead of
        // ending in a hard needle point.
        let w = mix(0.45 * cw, 0.010, r.y) * (1.0 + 1.2 * r.y);
        let c_cov = 1.0 - smoothstep(w * 0.5, w, r.x);
        if (c_cov > cover) {
            cover = c_cov;
            tipf = r.y;
        }
    }
    return vec2<f32>(cover, tipf);
}

// One parallax layer of fireflies added to `col`. Cells drift (slow rise + a
// gentle horizontal wander) and carry the canvas pan by depth. Each live cell is
// a warm dot with a soft halo that blinks with a sharp breathing pulse.
fn firefly_layer(col: vec3<f32>, uv: vec2<f32>, pan: vec2<f32>, time: f32,
                 i: i32, density: f32, glow: f32, pulse: f32) -> vec3<f32> {
    let depth = 1.0 + f32(i) * 0.8;
    let scale = 3.6 / depth;
    let drift = vec2<f32>(sin(time * 0.06 + f32(i) * 1.7) * 0.4, -time * 0.02 * depth);
    let sp = uv * scale + pan * 0.0008 * depth + drift;
    let id = floor(sp);
    let f = fract(sp) - 0.5;
    let h = hash(id + f32(i) * 29.0);
    if (h > 1.0 - 0.10 * density) {
        let ph = hash(id + f32(i) * 7.0) * 6.2831853;
        let br = 0.5 + 0.5 * sin(time * pulse + ph);
        let pulsev = pow(br, 3.0);                     // mostly-off blink
        let wob = vec2<f32>(sin(time * 0.8 + ph), cos(time * 1.1 + ph)) * 0.16;
        let d = length(f - wob);
        let core = smoothstep(0.05, 0.0, d);
        let halo = exp(-d * d * 34.0) * 0.6;
        let warm = mix(vec3<f32>(1.0, 0.72, 0.28), vec3<f32>(0.82, 1.0, 0.42), fract(h * 17.0));
        return col + warm * (core + halo) * pulsev * glow / depth;
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
    let firefly_density = pc.params[0].y;
    let glow = pc.params[0].z;
    let pulse = pc.params[0].w;
    let grass_height = pc.params[1].x;
    let vignette = pc.params[1].y;
    let vig_radius = pc.params[1].z;
    let vig_softness = pc.params[1].w;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    // Dark green-blue night column: deep teal at the top sinking to a warmer
    // meadow green toward the ground.
    let t = frag.y / res.y;
    var col = mix(vec3<f32>(0.015, 0.055, 0.075), vec3<f32>(0.02, 0.075, 0.06), t);

    // A faint warm ground mist, densest near the base, drifting very slowly.
    let mist_uv = uv * 1.2 + pan * 0.0002 + flow * 0.0003 + vec2<f32>(time * 0.01 * drift, 0.0);
    let m = fbm(mist_uv);
    let ground_bias = smoothstep(0.0, 0.55, screen_uv.y + 0.5);
    col = col + mix(vec3<f32>(0.06, 0.09, 0.05), vec3<f32>(0.02, 0.05, 0.04), m) * pow(m, 1.6) * 0.3 * ground_bias;

    // Fireflies behind the grass (far depth layers), warmly lifting the mid air.
    col = firefly_layer(col, uv, pan, time, 2, firefly_density, glow, pulse * 1.3);
    col = firefly_layer(col, uv, pan, time, 1, firefly_density, glow, pulse * 1.1);

    // Grass silhouettes, far → near (near drawn last, on top). Sway rides the
    // drift knob; the near layer is the darkest and tallest. Each layer's tips
    // are faded into the air by `tip_frac` (atmospheric perspective), so the top
    // of the field melts into the night instead of reading as a hard spiky edge;
    // the far layer fades most, the near layer keeps a little more body.
    let sway = 0.12 * sin(time * 0.5 * drift);
    let g_far = grass_cover(screen_uv + vec2<f32>(pan.x * 0.0004, 0.0), 0.44, 0.16 * grass_height, 70.0, sway * 0.6, 3.0);
    col = mix(col, vec3<f32>(0.030, 0.095, 0.085), g_far.x * (1.0 - 0.58 * g_far.y));
    let g_mid = grass_cover(screen_uv + vec2<f32>(pan.x * 0.0007, 0.0), 0.50, 0.26 * grass_height, 52.0, sway * 0.85, 11.0);
    col = mix(col, vec3<f32>(0.016, 0.062, 0.048), g_mid.x * (1.0 - 0.46 * g_mid.y));
    let g_near = grass_cover(screen_uv + vec2<f32>(pan.x * 0.0011, 0.0), 0.58, 0.40 * grass_height, 38.0, sway, 23.0);
    col = mix(col, vec3<f32>(0.008, 0.034, 0.026), g_near.x * (1.0 - 0.36 * g_near.y));

    // A near layer of fireflies in front of the grass for depth.
    col = firefly_layer(col, uv, pan, time, 0, firefly_density, glow, pulse);

    // Lock-screen ease: settle the meadow into a darker, stiller night.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.45 + vec3<f32>(0.006, 0.014, 0.01), l);

    // Optional edge vignette in zoom-independent screen space.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

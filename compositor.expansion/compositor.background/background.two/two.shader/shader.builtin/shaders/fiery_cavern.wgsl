// Built-in background: "Fiery Cavern" — the *inside* companion to Fiery Galaxy.
// Instead of molten worlds seen from orbit, you sit within a dark cavern of the
// fiery world: rock silhouettes framing a slow warm glow, a low seam of molten
// rock, and embers drifting up. Same quiet mood and the same Push/`@prop` contract.
//
// Design notes (kept deliberately restful):
//   * Mostly dark — a near-black cavern with a soft warm glow rising from below.
//     The heat is ambient, not a glare.
//   * Dark rock silhouettes: stalactites from the ceiling and a foreground ledge,
//     their lava-facing edges catching a thin warm rim. They parallax by depth.
//   * A low molten seam with a slow-drifting crust, and embers rising and fading —
//     the only bright accents, all gentle and slow.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Drift & embers" group="Cavern"
// @prop ember_density float default=1.0 min=0.0 max=2.0 label="Ember density" group="Cavern"
// @prop ember float default=1.0 min=0.0 max=2.0 label="Glow intensity" group="Cavern"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Cavern"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Cavern"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Cavern"

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

fn ridgeline(x: f32, seed: f32) -> f32 {
    let a = fbm(vec2<f32>(x * 0.9, seed));
    let b = fbm(vec2<f32>(x * 2.6 + 3.0, seed));
    return (a * 0.7 + b * 0.3 - 0.5) * 1.3;
}

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
    let ember_density = pc.params[0].y;
    let glow_amt = pc.params[0].z;
    let vignette = pc.params[0].w;
    let vig_radius = pc.params[1].x;
    let vig_softness = pc.params[1].y;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let vy = screen_uv.y;                          // +top (ceiling) .. −bottom (lava)

    // Dark cavern with a soft warm glow rising from the molten lake below.
    let dark = vec3<f32>(0.018, 0.012, 0.013);
    var col = dark + vec3<f32>(0.26, 0.10, 0.03) * smoothstep(0.4, -0.4, vy) * (0.35 + 0.3 * glow_amt);
    // A gentle flicker in the ambient glow (very slow, so it breathes not strobes).
    let flick = 0.92 + 0.08 * sin(time * 0.7) * sin(time * 0.31 + 1.3);
    col = col * flick;

    // The molten lake, a muted glowing band low in the frame: a wavering surface
    // broken up by a slow-drifting dark crust, so it reads as cooling rock, not a
    // bright pool.
    let seam = -0.27 + 0.035 * fbm(vec2<f32>(uv.x * 1.6 + pan.x * 0.0004, 0.0));
    // Soft bloom hugging the lake surface.
    col = col + vec3<f32>(0.5, 0.20, 0.05) * exp(-(vy - seam) * (vy - seam) * 55.0) * (0.4 + 0.4 * glow_amt);
    let below = smoothstep(0.012, -0.012, vy - seam);
    if (below > 0.0) {
        // Dark cooling crust with bright veins of lava flowing across the surface.
        let fx = vec2<f32>(uv.x * 2.2 + time * 0.04 * drift, vy * 3.0 - time * 0.03 * drift);
        var lakecol = mix(vec3<f32>(0.05, 0.021, 0.013), vec3<f32>(0.12, 0.05, 0.02), fbm(fx)) * (0.5 + 0.3 * glow_amt);
        let vein1 = smoothstep(0.03, 0.0, abs(fbm(fx * 1.3 + 7.0) - 0.5));
        let vein2 = smoothstep(0.02, 0.0, abs(fbm(fx * 2.6 - 3.0) - 0.5)) * 0.6;
        let heat = clamp(vein1 + vein2, 0.0, 1.0) * (0.7 + 0.5 * glow_amt);
        lakecol = lakecol + mix(vec3<f32>(0.7, 0.28, 0.06), vec3<f32>(1.0, 0.68, 0.24), heat) * heat;
        col = mix(col, lakecol, below);
    }

    // A near foreground rock lip at the very bottom (dark silhouette), its top edge
    // rim-lit by the lake it fronts.
    let lip = -0.42 + 0.09 * ridgeline(uv.x + pan.x * 0.0004, 5.0);
    let onlip = smoothstep(0.008, -0.008, vy - lip);
    let lip_rim = smoothstep(0.0, 0.04, lip - vy) * smoothstep(0.12, 0.0, lip - vy);
    col = mix(col, vec3<f32>(0.02, 0.012, 0.011), onlip);
    col = col + vec3<f32>(0.9, 0.36, 0.09) * lip_rim * onlip * (0.6 + 0.5 * glow_amt);

    // Stalactites hanging from the ceiling (dark) against the glow — a farther
    // layer that parallaxes more; their tips faintly catch the light.
    let ceil = 0.42 - 0.26 * abs(ridgeline(uv.x + pan.x * 0.0011, 33.0));
    let onceil = smoothstep(0.012, -0.012, ceil - vy);
    col = mix(col, vec3<f32>(0.012, 0.008, 0.010), onceil);
    let tip = smoothstep(0.0, 0.05, vy - ceil) * smoothstep(0.14, 0.0, vy - ceil);
    col = col + vec3<f32>(0.45, 0.17, 0.05) * tip * onceil * 0.5 * glow_amt;

    // Enclosing rock walls down the left and right edges — an irregular dark frame
    // that makes the cavern feel enclosed and catches a warm rim near the lake.
    let wallx = abs(screen_uv.x);
    let wedge = 0.66 + 0.09 * fbm(vec2<f32>(vy * 3.5 + 2.0, sign(screen_uv.x) * 4.0));
    let wall = smoothstep(wedge, wedge + 0.10, wallx);
    col = mix(col, vec3<f32>(0.014, 0.009, 0.009), wall * 0.9);
    let wrim = smoothstep(wedge + 0.05, wedge, wallx) * smoothstep(wedge - 0.07, wedge, wallx);
    col = col + vec3<f32>(0.4, 0.16, 0.05) * wrim * smoothstep(0.2, -0.45, vy) * 0.5 * glow_amt;

    // Embers rising from the seam, swaying and fading as they climb.
    for (var i = 0; i < 3; i = i + 1) {
        let depth = 1.0 + f32(i) * 0.6;
        let t = time * 0.06 * drift / depth;
        let sp = vec2<f32>(uv.x * (8.0 / depth) + pan.x * 0.001 * depth + sin(time * 0.4 + f32(i) + uv.y * 3.0) * 0.15,
                           uv.y * (8.0 / depth) - t + pan.y * 0.001 * depth);
        let id = floor(sp);
        let fp = fract(sp) - 0.5;
        let h = hash(id + f32(i) * 27.0);
        if (h > 1.0 - 0.05 * ember_density) {
            let dd = length(fp);
            let rise = smoothstep(-0.55, 0.4, vy);        // brightest low, fading up
            let tw = 0.5 + 0.5 * sin(time * 1.5 + h * 30.0);
            col = col + vec3<f32>(1.0, 0.45, 0.12) * smoothstep(0.08, 0.0, dd) * tw * (1.0 - rise) / (depth * 2.6);
        }
    }

    // Lock-screen ease: let the fire settle to a stiller, dimmer glow.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.55 + vec3<f32>(0.01, 0.004, 0.003), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

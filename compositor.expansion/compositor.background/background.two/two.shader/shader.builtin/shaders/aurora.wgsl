// Built-in background: "Aurora" — vertical curtains of northern light over a quiet
// starfield. Sine-warped ribbons rise from the horizon with an additive green/teal/
// purple glow. Very high impact for low cost, and it pairs naturally with the same
// twinkling parallax stars used by the stock parallax. Same Push / `@prop` contract.
//
// Design notes (kept deliberately restful):
//   * A deep night gradient (indigo → near-black) with a faint airglow at the base.
//   * Three parallax layers of sparse, twinkling stars (the parallax starfield).
//   * The aurora is a stack of vertical curtains. Each curtain is a horizontal band
//     of light whose position is warped by layered sines (the shimmer) and whose
//     brightness rises from the horizon and feathers out toward the top of the sky.
//     Fine vertical striations (the "rays") are carved in with a high-frequency
//     noise so each curtain reads as folds of light, not a flat wash. Additive.
//   * The `hue` knob slides the palette from green/teal to teal/violet/magenta; the
//     curtains drift slowly and lean with the canvas flow.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop aurora_speed float default=1.0 min=0.0 max=4.0 label="Shimmer speed" group="Aurora"
// @prop intensity float default=1.0 min=0.0 max=2.0 label="Intensity" group="Aurora"
// @prop star_density float default=1.0 min=0.0 max=2.0 label="Star density" group="Aurora"
// @prop hue float default=0.35 min=0.0 max=1.0 label="Green → violet" group="Aurora"
// @prop altitude float default=0.0 min=-0.5 max=0.6 label="Curtain height" group="Aurora"
// @prop vignette float default=0.25 min=0.0 max=1.0 label="Vignette amount" group="Frame"
// @prop vignette_radius float default=1.2 min=0.5 max=2.0 label="Vignette radius" group="Frame"
// @prop vignette_softness float default=0.7 min=0.05 max=2.0 label="Vignette softness" group="Frame"

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
    for (var i = 0; i < 4; i = i + 1) {
        v = v + a * noise(p);
        p = p * 2.0;
        a = a * 0.5;
    }
    return v;
}

// Sparse twinkling stars, three parallax depths — the same field the stock parallax
// uses, so the two backgrounds feel related.
fn stars(uv: vec2<f32>, pan: vec2<f32>, time: f32, density: f32) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    for (var i = 1; i <= 3; i = i + 1) {
        let depth = f32(i) * 0.5;
        let sp = uv * (45.0 / depth) + pan * 0.001 * depth;
        let id = floor(sp);
        let fp = fract(sp) - 0.5;
        let h = hash(id);
        if (h > 1.0 - 0.04 * density) {
            let twink = 0.5 + 0.5 * sin(time * 1.5 + h * 50.0);
            let dd = length(fp);
            let sc = mix(vec3<f32>(0.7, 0.9, 1.0), vec3<f32>(0.85, 0.8, 1.0), fract(h * 133.7));
            let glow = smoothstep(0.06, 0.0, dd) + smoothstep(0.2, 0.0, dd) * 0.3;
            c = c + sc * glow * twink / depth;
        }
    }
    return c;
}

// The aurora palette: green → teal → violet → magenta as `t` goes 0..1, biased by
// the author's `hue` knob.
fn aurora_ramp(t: f32) -> vec3<f32> {
    let green = vec3<f32>(0.20, 0.95, 0.55);
    let teal = vec3<f32>(0.15, 0.80, 0.80);
    let violet = vec3<f32>(0.45, 0.35, 0.95);
    let magenta = vec3<f32>(0.85, 0.35, 0.80);
    let a = mix(green, teal, smoothstep(0.0, 0.5, t));
    let b = mix(violet, magenta, smoothstep(0.5, 1.0, t));
    return mix(a, b, smoothstep(0.35, 0.75, t));
}

// One curtain of light. `k` seeds its shape/phase; the band's centre height is warped
// by layered sines (the shimmer) and it glows brightest just above the horizon.
fn curtain(uv: vec2<f32>, base_y: f32, k: f32, time: f32, speed: f32, flow_x: f32) -> f32 {
    // Horizontal warp: where along x this fold of light sits, wobbling over time.
    let phase = time * 0.25 * speed;
    let warp = 0.16 * sin(uv.x * 1.3 + phase + k * 6.283)
             + 0.09 * sin(uv.x * 2.7 - phase * 1.4 + k * 12.0)
             + 0.04 * sin(uv.x * 5.1 + phase * 0.7);
    let centre = base_y + warp + flow_x * 0.0004;
    // Vertical falloff: a soft band, fatter at the base, feathering up into the sky.
    let dy = uv.y - centre;
    let band = exp(-dy * dy * 26.0) + 0.35 * exp(-max(dy, 0.0) * 3.2);
    // Vertical striations (the rays), scrolling sideways so the curtain shimmers.
    let rays = 0.55 + 0.45 * fbm(vec2<f32>(uv.x * 7.0 + phase * 2.0 + k * 30.0, uv.y * 1.5));
    // Fade the whole curtain in from the horizon and out at the top of the frame.
    // The upper edge is kept high so curtains fanned into the sky still glow.
    let horizon = smoothstep(-0.7, -0.2, uv.y) * smoothstep(1.05, 0.15, uv.y);
    return band * rays * horizon;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let frag = in.pos.xy;
    let res = pc.res_zoom_time.xy;
    let zoom = pc.res_zoom_time.z;
    let time = pc.res_zoom_time.w;
    let pan_in = pc.pan_flow.xy;
    let flow_in = pc.pan_flow.zw;
    let lock_amount = pc.lock_alpha.x;
    let alpha = pc.lock_alpha.y;

    let speed = pc.params[0].x;
    let intensity = pc.params[0].y;
    let star_density = pc.params[0].z;
    let hue = clamp(pc.params[0].w, 0.0, 1.0);
    let altitude = pc.params[1].x;
    let vignette = pc.params[1].y;
    let vig_radius = pc.params[1].z;
    let vig_softness = pc.params[1].w;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let flow = vec2<f32>(flow_in.x, -flow_in.y);

    // Night sky: indigo overhead deepening to near-black, faint airglow at the base.
    let h = clamp(screen_uv.y * 0.6 + 0.5, 0.0, 1.0);
    var col = mix(vec3<f32>(0.02, 0.03, 0.07), vec3<f32>(0.01, 0.01, 0.03), h);
    col = col + vec3<f32>(0.02, 0.06, 0.05) * smoothstep(-0.2, -0.7, screen_uv.y);

    col = col + stars(uv, pan, time, star_density);

    // A curtain of light spread across the whole sky rather than one fixed strip.
    // Each curtain sits at its own height (fanned from just above the horizon up
    // into the sky) and slowly rises and falls on an independent phase, so the
    // aurora drifts and breathes instead of clamping to a single band. Additive so
    // overlaps bloom brighter, the way real aurora layers do.
    let CURTAINS = 5;
    for (var i = 0; i < CURTAINS; i = i + 1) {
        let fi = f32(i) / f32(CURTAINS - 1);
        let k = f32(i) * 0.37 + fi * 0.9;
        let spread = mix(-0.16, 0.40, fi);
        let drift = 0.10 * sin(time * 0.13 * speed + k * 2.3)
                  + 0.05 * sin(time * 0.07 * speed - k * 1.7);
        let base_y = altitude + spread + drift + 0.04 * sin(k * 4.0);
        let g = curtain(uv, base_y, k, time, speed, flow.x);
        let band_hue = clamp(hue + 0.22 * fi - 0.10, 0.0, 1.0);
        let tint = aurora_ramp(band_hue);
        col = col + tint * g * 0.42 * intensity;
    }

    // Lock-screen ease: calm the shimmer and deepen the night.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.6 + vec3<f32>(0.004, 0.006, 0.014), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

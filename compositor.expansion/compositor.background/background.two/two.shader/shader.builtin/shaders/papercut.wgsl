// Built-in background: "Paper-cut Layers" — concentric wavy bands in a soft
// monochrome ramp, like sheets of layered card stock seen from above. A companion
// to the other built-in scenes: same quiet, low-contrast mood, same Push/`@prop`
// contract, so it slots into the built-in shader list and the live preview.
//
// Design notes (deliberately smooth — zero noise grain):
//   * Concentric rings around a slowly drifting centre, their radius modulated by
//     a sum of smooth sines so each boundary undulates like a hand-cut edge.
//   * The radius is quantised into flat bands (the "cards"), shaded from a single
//     desaturated hue: bright near the centre, receding to dark at the rim.
//   * A soft inner shadow sits just inside each band's edge, the cue that one
//     card overlaps the next — the whole depth read, with no texture or noise.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop layers float default=9.0 min=3.0 max=24.0 label="Layer count" group="Paper-cut"
// @prop amplitude float default=1.0 min=0.0 max=2.0 label="Wave amplitude" group="Paper-cut"
// @prop detail float default=3.0 min=1.0 max=8.0 label="Wave detail" group="Paper-cut"
// @prop drift_speed float default=1.0 min=0.0 max=4.0 label="Drift speed" group="Paper-cut"
// @prop hue float default=0.6 min=0.0 max=1.0 label="Hue" group="Paper-cut"
// @prop shadow float default=1.0 min=0.0 max=2.0 label="Edge shadow" group="Paper-cut"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Paper-cut"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Paper-cut"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Paper-cut"

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
// the shader once the engine gains a texture descriptor for pixel programs. This
// scene is textureless by design, so the helper below is staked out but unused.
//
// `params[3]` is reserved as the sprite-sheet control vec4 (zero-filled today):
//   params[3].x = atlas columns        params[3].z = playback fps
//   params[3].y = atlas rows           params[3].w = frame count (0 = cols*rows)
//
// When a texture arrives, bind it here and switch onto the helper:
//   @group(0) @binding(0) var atlas_tex: texture_2d<f32>;
//   @group(0) @binding(1) var atlas_smp: sampler;
//
// Sub-rect UV for the current animation frame of a cols×rows sheet. `cell` is the
// 0..1 coord within one sprite.
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

// A single desaturated hue from the wheel — smooth, muted, card-stock coloured.
fn hue_color(h: f32) -> vec3<f32> {
    let c = 0.5 + 0.5 * cos(6.2831853 * (vec3<f32>(h) + vec3<f32>(0.0, 0.33, 0.67)));
    let lum = dot(c, vec3<f32>(0.299, 0.587, 0.114));
    return mix(vec3<f32>(lum), c, 0.6);              // pull 40% toward grey
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

    let layers = max(pc.params[0].x, 1.0);
    let amplitude = pc.params[0].y;
    let detail = max(pc.params[0].z, 1.0);
    let drift = pc.params[0].w;
    let hue = pc.params[1].x;
    let shadow = pc.params[1].y;
    let vignette = pc.params[1].z;
    let vig_radius = pc.params[1].w;
    let vig_softness = pc.params[2].x;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    // Slowly drifting centre so the whole stack breathes with the canvas.
    let center = vec2<f32>(0.0, 0.12)
        + pan * 0.0004
        + flow * 0.0004
        + vec2<f32>(sin(time * 0.05 * drift), cos(time * 0.037 * drift)) * 0.06;

    let rel = uv - center;
    let d = length(rel);
    let ang = atan2(rel.y, rel.x);

    // Smooth wavy modulation of the ring radius (sum of a few sines — no noise).
    let wob = amplitude * 0.12 * (
        0.6 * sin(ang * detail + time * 0.08 * drift)
        + 0.3 * sin(ang * (detail * 2.0 + 1.0) - time * 0.05 * drift)
        + 0.15 * sin(ang * (detail * 3.0 + 2.0)));
    let r = d + wob;

    // Quantise the radius into flat cards; `edge` is the 0..1 sweep within a band.
    let steps = r * layers;
    let band = floor(steps);
    let edge = steps - band;

    // Monochrome ramp: bright at the centre receding to dark at the rim.
    let band_norm = clamp(band / (layers * 1.4), 0.0, 1.0);
    let lightness = mix(0.95, 0.28, band_norm);

    // Soft inner shadow just inside each band's outer edge — the overlap cue.
    let lip = 1.0 - smoothstep(0.0, 0.12, edge);
    var col = hue_color(hue) * lightness * (1.0 - 0.35 * clamp(shadow, 0.0, 2.0) * lip);

    // A faint cool-to-warm vertical tint so the flat stack has a touch of air.
    let vt = frag.y / res.y;
    col = col * mix(vec3<f32>(1.02, 1.0, 1.03), vec3<f32>(0.98, 0.99, 0.97), vt);

    // Lock-screen ease: settle into a darker, stiller card.
    var lk = clamp(lock_amount, 0.0, 1.0);
    lk = lk * lk * (3.0 - 2.0 * lk);
    col = mix(col, col * 0.5, lk);

    // Optional edge vignette in zoom-independent screen space.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

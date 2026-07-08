// Built-in background: "Ribbons" — stacked sine bands sliding across the frame,
// each layer a step darker in a muted palette, their phases drifting so the seams
// between them rise and fall like slow silk. Fully abstract cousin of the mountain
// look, but with no horizon and no hard edge. Same Push/`@prop` contract as the
// stock parallax so it slots into the built-in picker + preview.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=3.0 label="Drift speed" group="Ribbons"
// @prop ribbon_count float default=6.0 min=2.0 max=10.0 label="Ribbon count" group="Ribbons"
// @prop amplitude float default=1.0 min=0.2 max=2.5 label="Wave amplitude" group="Ribbons"
// @prop softness float default=1.0 min=0.2 max=3.0 label="Edge softness" group="Ribbons"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Ribbons"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Ribbons"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Ribbons"

struct Push {
    res_zoom_time: vec4<f32>,        // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,             // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,           // x = lock_amount, y = alpha
    params: array<vec4<f32>, 4>,     // shader-authored @prop values (16 floats)
};
var<immediate> pc: Push;

// ── Reserved: texture / animated sprite-sheet slot (not yet wired) ────────────
// The renderer drives background shaders with push constants only — there is no
// bound texture or sampler in the dispatch seam yet. This block stakes out the
// contract (matching underwater.wgsl) so a sprite-sheet atlas can be dropped in
// without reworking the shader once pixel programs gain a texture descriptor.
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

fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}

// The signed vertical distance from `uv.y` to ribbon `i`'s crest line: a sum of
// two sines at different wavelengths so the crest is organic, not a pure wave.
fn crest(x: f32, i: f32, t: f32, amp: f32) -> f32 {
    let ph = i * 1.7;
    let base = -0.55 + i * 0.16;   // stacked baseline, bottom → top
    let w = amp * (0.10 + 0.02 * hash(vec2<f32>(i, 2.0)));
    return base
        + w * sin(x * 1.3 + t * (0.6 + 0.15 * i) + ph)
        + w * 0.5 * sin(x * 2.7 - t * (0.4 + 0.1 * i) + ph * 1.6);
}

// The muted per-layer palette: a cool deep band that warms slightly toward the
// front layers. `s` is the layer's normalised index.
fn layer_color(s: f32) -> vec3<f32> {
    let back = vec3<f32>(0.020, 0.040, 0.060);
    let front = vec3<f32>(0.075, 0.060, 0.090);
    return mix(back, front, s);
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
    let ribbon_count = pc.params[0].y;
    let amplitude = pc.params[0].z;
    let softness = pc.params[0].w;
    let vignette = pc.params[1].x;
    let vig_radius = pc.params[1].y;
    let vig_softness = pc.params[1].z;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let x = uv.x - pan.x * 0.00035 - flow.x * 0.0004;
    let y = uv.y - pan.y * 0.00020;

    let t = time * drift * 0.2;

    // Deep base sky behind every ribbon.
    var col = mix(vec3<f32>(0.010, 0.018, 0.030), vec3<f32>(0.016, 0.014, 0.028), frag.y / res.y);

    // Painter's algorithm back-to-front: each ribbon fills everything below its
    // crest, so nearer (higher-index) ribbons overlay the ones behind. Soft edge
    // via smoothstep across the crest line; width tracks zoom through `fwidth`.
    let n = i32(clamp(ribbon_count, 2.0, 10.0));
    let aa = fwidth(y) * (2.0 + 6.0 * softness);
    for (var i = 0; i < 10; i = i + 1) {
        if (i >= n) { break; }
        let fi = f32(i);
        let cy = crest(x, fi, t, amplitude);
        let cover = smoothstep(cy + aa, cy - aa, y);   // 1 below crest, 0 above
        let s = fi / max(1.0, f32(n - 1));
        var lc = layer_color(s);
        // A faint sheen just under each crest gives the silk highlight.
        let sheen = smoothstep(cy - aa, cy - 0.05, y) * (1.0 - smoothstep(cy - 0.05, cy - 0.15, y));
        lc = lc + vec3<f32>(0.04, 0.05, 0.07) * sheen * 0.6;
        col = mix(col, lc, cover);
    }

    // Lock-screen ease: settle into a darker, stiller state.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.004, 0.008, 0.012), l);

    // Optional edge vignette in zoom-independent screen space.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

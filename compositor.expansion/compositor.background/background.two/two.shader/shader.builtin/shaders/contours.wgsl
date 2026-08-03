// Built-in background: "Contours" — topographic isolines drawn from a slowly
// morphing noise height-field: thin, evenly-spaced elevation lines on a near-black
// ground, like a survey map breathing. Technical and quiet; the lines are so fine
// and low-contrast they never pull the eye off the foreground. Same Push/`@prop`
// contract as the stock parallax so it slots into the built-in picker + preview.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=3.0 label="Drift speed" group="Contours"
// @prop line_count float default=14.0 min=4.0 max=40.0 label="Line count" group="Contours"
// @prop thickness float default=1.0 min=0.3 max=3.0 label="Line thickness" group="Contours"
// @prop tint float default=1.0 min=0.0 max=2.0 label="Elevation tint" group="Contours"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Contours"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Contours"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Contours"

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

// `hash` is a pure function of two fract lanes: the vec3's z lane always
// duplicates x, since both are `fract(p.x * 0.1031)`. Splitting the lane setup
// out lets `noise` compute its four corner lanes once and share them across all
// four taps. Every operand and operation order below is unchanged, so the output
// is bit-identical to the vec3 form this replaced.
fn hash_lane(x: f32, y: f32) -> f32 {
    let p3 = vec3<f32>(x, y, x);
    let d = dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract(((x + d) + (y + d)) * (x + d));
}
fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * 0.1031);
    return hash_lane(q.x, q.y);
}
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    // One vec4 mul+fract for all four corner lanes, each still computed exactly as
    // the four separate `hash` calls did: fract(i.x*K), fract((i.x+1)*K),
    // fract(i.y*K), fract((i.y+1)*K).
    let q = fract(vec4<f32>(i.x, i.x + 1.0, i.y, i.y + 1.0) * 0.1031);
    return mix(
        mix(hash_lane(q.x, q.z), hash_lane(q.y, q.z), f.x),
        mix(hash_lane(q.x, q.w), hash_lane(q.y, q.w), f.x),
        f.y,
    );
}
// Octave count for `fbm`. The loader rewrites this literal to the `@optimized`
// value when the world's Optimized toggle is on, and naga folds it through the
// loop below — one source, two compiled variants, nothing to keep in step.
const FBM_OCTAVES: i32 = 5;   // @optimized 2
// The reference walk's amplitude sum. `fbm` renormalises to it so a shortened
// walk keeps this scene's brightness instead of coming out dim. At the reference
// count the factor is exactly 1.0 — both values are exact binary fractions — so
// the reference output is bit-identical to the fixed-5-octave version.
const FBM_REF_SUM: f32 = 0.96875;
fn fbm(p_in: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var norm = 0.0;
    var p = p_in;
    for (var i = 0; i < FBM_OCTAVES; i = i + 1) {
        v = v + a * noise(p);
        norm = norm + a;
        p = p * 2.0;
        a = a * 0.5;
    }
    return v * (FBM_REF_SUM / norm);
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
    let line_count = pc.params[0].y;
    let thickness = pc.params[0].z;
    let tint = pc.params[0].w;
    let vignette = pc.params[1].x;
    let vig_radius = pc.params[1].y;
    let vig_softness = pc.params[1].z;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    uv = uv - pan * 0.00035 - flow * 0.0004;

    let t = time * drift * 0.05;

    // The height-field: a couple of octaves that slowly slide against each other so
    // the isolines advance and merge like a shifting landscape, never scrolling.
    let h = fbm(uv * 1.6 + vec2<f32>(t * 0.4, -t * 0.25))
          + 0.5 * fbm(uv * 3.1 - vec2<f32>(t * 0.2, t * 0.3));

    // Isolines = where the scaled height crosses an integer. `fwidth` gives the
    // per-pixel slope so the line stays one screen-pixel wide at any zoom (proper
    // analytic AA, resolution-independent).
    let e = h * line_count;
    let d = abs(fract(e - 0.5) - 0.5);          // distance to nearest contour, in e-units
    let w = fwidth(e) * (0.8 * thickness);
    let line = 1.0 - smoothstep(0.0, w, d);

    // Near-black ground with a faint elevation tint so higher bands read a touch
    // warmer — barely perceptible, keeps the map from looking monochrome.
    let band = fract(h * 0.5);
    let ground = mix(vec3<f32>(0.010, 0.020, 0.028), vec3<f32>(0.020, 0.028, 0.024),
                     band * clamp(tint, 0.0, 2.0) * 0.5);
    let ink = mix(vec3<f32>(0.16, 0.30, 0.34), vec3<f32>(0.22, 0.34, 0.26),
                  band) * (0.5 + 0.5 * tint);

    var col = mix(ground, ink, line);

    // Lock-screen ease: settle into a darker, stiller state.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.004, 0.008, 0.010), l);

    // Optional edge vignette in zoom-independent screen space.
    // Guarded because the default is off: the length() and the smoothstep would
    // otherwise run on every pixel only to be multiplied by 1.0. The guard also
    // makes a 0 softness safe — that smoothstep divides by zero, and
    // mix(1.0, NaN, 0.0) is NaN, not 1.0.
    let vig_amount = clamp(vignette, 0.0, 1.0);
    if (vig_amount > 0.0) {
        let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
        col = col * mix(1.0, vig, vig_amount);
    }
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

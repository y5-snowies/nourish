// Built-in background: "Aurora Blur" — three or four enormous, heavily-blurred
// colour blobs orbiting behind a dark veil, the calm "desktop wallpaper" look.
// The most neutral of the abstract set: no shapes to read, no motion to track,
// just broad fields of muted colour sliding past each other. Same Push/`@prop`
// contract as the stock parallax so it slots into the built-in picker + preview.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=3.0 label="Drift speed" group="Aurora"
// @prop saturation float default=1.0 min=0.0 max=1.8 label="Saturation" group="Aurora"
// @prop intensity float default=1.0 min=0.3 max=2.0 label="Intensity" group="Aurora"
// @prop grain float default=1.0 min=0.0 max=2.0 label="Grain" group="Aurora"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Aurora"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Aurora"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Aurora"

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

// One huge soft blob: a Gaussian falloff so wide it never shows an edge, only a
// slow rise toward its drifting centre. `tint` is pre-desaturated by the caller.
fn blob(uv: vec2<f32>, center: vec2<f32>, sigma: f32, tint: vec3<f32>) -> vec3<f32> {
    let d = uv - center;
    let g = exp(-dot(d, d) / (2.0 * sigma * sigma));
    return tint * g;
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
    let saturation = pc.params[0].y;
    let intensity = pc.params[0].z;
    let grain = pc.params[0].w;
    let vignette = pc.params[1].x;
    let vig_radius = pc.params[1].y;
    let vig_softness = pc.params[1].z;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    uv = uv - pan * 0.00030 - flow * 0.0004;

    let t = time * drift * 0.08;

    // A muted, dark base — cool navy at the top easing to a deep plum below.
    var col = mix(vec3<f32>(0.030, 0.035, 0.060), vec3<f32>(0.050, 0.030, 0.055), frag.y / res.y);

    // Four broad blobs on slow, differently-phased orbits. Each tint is pulled
    // toward its own luminance by the saturation knob so the default reads soft.
    var tints = array<vec3<f32>, 4>(
        vec3<f32>(0.18, 0.10, 0.28),   // violet
        vec3<f32>(0.08, 0.20, 0.26),   // teal
        vec3<f32>(0.24, 0.12, 0.16),   // rose
        vec3<f32>(0.10, 0.16, 0.30),   // indigo
    );
    for (var i = 0; i < 4; i = i + 1) {
        let fi = f32(i);
        let s = hash(vec2<f32>(fi, 3.0));
        let center = vec2<f32>(
            sin(t * (0.6 + 0.3 * s) + fi * 1.9) * (0.55 + 0.15 * s),
            cos(t * (0.5 + 0.25 * s) + fi * 2.7) * 0.45,
        );
        let sigma = 0.55 + 0.25 * s;
        var tint = tints[i];
        // Desaturate toward the tint's own luminance as saturation drops.
        let lum = dot(tint, vec3<f32>(0.299, 0.587, 0.114));
        tint = mix(vec3<f32>(lum), tint, saturation);
        col = col + blob(uv, center, sigma, tint) * intensity;
    }

    // A whisper of ordered-ish grain to break up banding on smooth gradients. The
    // hash is per-pixel and static-ish; kept very low so it never sparkles.
    let g = (hash(frag) - 0.5) * 0.012 * grain;
    col = col + vec3<f32>(g);

    // Lock-screen ease: settle into a darker, stiller state.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.006, 0.008, 0.014), l);

    // Optional edge vignette in zoom-independent screen space.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

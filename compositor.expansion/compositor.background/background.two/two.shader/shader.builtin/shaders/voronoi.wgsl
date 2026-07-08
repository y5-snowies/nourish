// Built-in background: "Voronoi" — a lattice of soft-edged cells with barely
// varying tints, drifting like frosted or stained glass held at low contrast. The
// cell seeds wander slowly so borders breathe without ever snapping. Same
// Push/`@prop` contract as the stock parallax so it slots into the built-in
// picker + preview.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=3.0 label="Drift speed" group="Voronoi"
// @prop scale float default=3.5 min=1.5 max=9.0 label="Cell scale" group="Voronoi"
// @prop edge float default=1.0 min=0.0 max=2.0 label="Edge darkness" group="Voronoi"
// @prop variation float default=1.0 min=0.0 max=2.0 label="Tint variation" group="Voronoi"
// @prop brightness float default=1.6 min=0.2 max=4.0 label="Brightness" group="Voronoi"
// @prop saturation float default=1.3 min=0.0 max=3.0 label="Vividness" group="Voronoi"
// @prop light float default=0.7 min=0.0 max=2.0 label="Light amount" group="Voronoi"
// @prop light_speed float default=1.0 min=0.0 max=3.0 label="Light drift" group="Voronoi"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Voronoi"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Voronoi"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Voronoi"

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
// A 2D cell-seed offset in [0,1)^2 for the cell at integer coord `c`.
fn hash2(c: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(hash(c), hash(c + vec2<f32>(37.2, 11.7)));
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
    let scale = pc.params[0].y;
    let edge = pc.params[0].z;
    let variation = pc.params[0].w;
    let brightness = pc.params[1].x;
    let saturation = pc.params[1].y;
    let light_amt = pc.params[1].z;
    let light_speed = pc.params[1].w;
    let vignette = pc.params[2].x;
    let vig_radius = pc.params[2].y;
    let vig_softness = pc.params[2].z;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    uv = uv - pan * 0.00035 - flow * 0.0004;

    let t = time * drift * 0.15;

    // Cellular scan over the 3×3 neighbourhood: track F1 (nearest seed) and F2
    // (second-nearest) so `F2 - F1` gives a clean soft border, and remember which
    // cell won so we can tint it. Seeds wobble on a slow per-cell orbit.
    let p = uv * scale;
    let cell = floor(p);
    let f = fract(p);
    var f1 = 8.0;
    var f2 = 8.0;
    var id = vec2<f32>(0.0);
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            let g = vec2<f32>(f32(i), f32(j));
            let seed = hash2(cell + g);
            // Slow orbit around the cell centre keeps borders alive without snapping.
            let o = 0.5 + 0.35 * sin(t + 6.2831 * seed + vec2<f32>(0.0, 1.57));
            let d = length(g + o - f);
            if (d < f1) {
                f2 = f1; f1 = d; id = cell + g;
            } else if (d < f2) {
                f2 = d;
            }
        }
    }

    // Per-cell tint: a hue wander across a richer teal→cyan→violet spread, scaled
    // by `variation`. The base sits well above black so the glass reads as lit
    // rather than murky; `saturation`/`brightness` push it the rest of the way.
    let ch = hash(id);
    let base = vec3<f32>(0.10, 0.17, 0.21);
    let warm = vec3<f32>(0.22, 0.12, 0.24);
    let cool = vec3<f32>(0.05, 0.22, 0.28);
    var col = base + mix(warm - base, cool - base, ch) * clamp(variation, 0.0, 2.0);

    // Soft border darkening where F1 ≈ F2 (equidistant to two seeds) — the leaded
    // seam of the glass. `edge` sets how dark and how wide.
    let border = 1.0 - smoothstep(0.0, 0.06 + 0.04 * edge, f2 - f1);
    col = col * (1.0 - 0.55 * edge * border);

    // A gentle radial fall toward each cell centre so cells read as slightly domed
    // panes rather than flat fills.
    col = col * (1.05 - 0.25 * smoothstep(0.0, 0.9, f1));

    // A soft light drifting across the glass in screen space, so cells nearer the
    // light glow while the panes stay domed. This is the "needs light" lift.
    let lt = time * 0.08 * light_speed;
    let light_pos = 0.38 * vec2<f32>(sin(lt), cos(lt * 0.83));
    let glow = smoothstep(1.05, 0.0, length(screen_uv - light_pos));
    // Specular catch on the cell dome, brightest at the pane centre.
    let dome = 1.0 - smoothstep(0.0, 0.7, f1);
    let light_col = vec3<f32>(0.55, 0.72, 0.85);
    col = col + light_col * (glow * (0.35 + 0.65 * dome)) * clamp(light_amt, 0.0, 2.0);

    // Vividness: push chroma away from its own luminance before exposure.
    let lum = dot(col, vec3<f32>(0.299, 0.587, 0.114));
    col = mix(vec3<f32>(lum), col, clamp(saturation, 0.0, 3.0));

    // Exposure. Boosts the whole image; the shader is no longer floor-dim.
    col = col * max(brightness, 0.0);

    // Lock-screen ease: settle into a darker, stiller state.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.004, 0.010, 0.012), l);

    // Optional edge vignette in zoom-independent screen space.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * alpha;
}

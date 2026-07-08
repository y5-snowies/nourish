// Built-in background: "Metaballs" — a screen-filling field of soft blobs that
// wander in place and fuse (smooth-min style) wherever they touch. Panning the
// world drags the field, and the *velocity* of that pan stretches each blob along
// the direction of travel, so fast movement smears neighbours together into
// connected channels; when you stop, they relax back into separate drifting cells.
// Fully abstract, biased dark, no hard sparkle — nothing competes with the
// foreground workspace. Uses the same Push/`@prop` contract as the stock
// parallax, so it drops straight into the built-in picker + live preview.
//
// Coverage note: blobs are placed on a jittered infinite grid (one wanderer per
// cell, sampled over the 3×3 neighbourhood of every fragment), so the field tiles
// and stays full-bleed no matter how far the world is panned or zoomed — never a
// bare corner. `blob_count` scales the grid density (few large / many small).
//
// Velocity note: the smoothed pan velocity arrives in the `lock_alpha.w` push
// lane as two snorm16 halves scaled by 16384 px/s (see draw.vulkan's
// `velocity_lane`; `lock_alpha.z` is the per-world sRGB flag). It is zero in the settings preview
// and whenever the world is still, so the stretch is a pure enhancement — the
// blobs already fill, wander and merge at rest.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=3.0 label="Drift speed" group="Metaballs"
// @prop blob_count float default=6.0 min=2.0 max=9.0 label="Blob density" group="Metaballs"
// @prop merge float default=1.0 min=0.2 max=2.5 label="Merge softness" group="Metaballs"
// @prop glow float default=1.0 min=0.0 max=2.0 label="Glow" group="Metaballs"
// @prop connect float default=1.0 min=0.0 max=2.0 label="Velocity connect" group="Metaballs"
// @prop hue float default=0.0 min=0.0 max=1.0 label="Palette shift" group="Metaballs"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Metaballs"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Metaballs"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Metaballs"

struct Push {
    res_zoom_time: vec4<f32>,        // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,             // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,           // x = lock_amount, y = alpha, z = srgb, w = packed pan velocity
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

// Driver-stable integer/bit-mix value hash (Dave Hoskins) — no `fract(sin)`, so
// the field stays box-free across Vulkan drivers (see the stock parallax note).
fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}

// Two independent hashes → a per-cell random vec2 (blob phase + radius jitter).
fn hash2(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(hash(p), hash(p + vec2<f32>(37.2, 11.7)));
}

// A monotone dark→teal→bright-crest walk: brightness rises with blob mass so the
// empty field stays near-black and the blob bodies read as the luminous element.
// `x` is the 0..1 blob mass (NOT wrapped — a wrap would paint a hard contour ring
// at every blob edge); `shift` slides the crest hue cool↔violet.
fn palette(x: f32, shift: f32) -> vec3<f32> {
    let a = vec3<f32>(0.014, 0.026, 0.040);   // near-black empty field
    let b = vec3<f32>(0.045, 0.100, 0.130);   // teal body
    let cool = vec3<f32>(0.115, 0.155, 0.205); // cool crest
    let violet = vec3<f32>(0.175, 0.110, 0.215); // violet crest
    let crest = mix(cool, violet, clamp(shift, 0.0, 1.0));
    let lo = mix(a, b, smoothstep(0.0, 0.55, x));
    return mix(lo, crest, smoothstep(0.5, 1.0, x));
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
    let pan_vel = unpack2x16snorm(bitcast<u32>(pc.lock_alpha.w)) * 16384.0;

    let drift = pc.params[0].x;
    let blob_count = pc.params[0].y;
    let merge = pc.params[0].z;
    let glow = pc.params[0].w;
    let connect = pc.params[1].x;
    let hue = pc.params[1].y;
    let vignette = pc.params[1].z;
    let vig_radius = pc.params[1].w;
    let vig_softness = pc.params[2].x;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    uv = uv - pan * 0.00035 - flow * 0.0004;

    let t = time * drift * 0.12;

    // Pan velocity → an elongation axis. Convert the world-space velocity into the
    // same uv units the pan uses, then map its speed through a saturating curve so
    // a brisk swipe stretches strongly while a slow drift barely nudges it. `k` is
    // the anisotropic reach along the axis; the perpendicular reach stays 1.0.
    let vel_uv = pan_vel * 0.00035 / max(zoom, 0.001);
    let speed = clamp(length(vel_uv) * 0.28, 0.0, 1.0) * connect;
    let axis = select(vec2<f32>(1.0, 0.0), normalize(vel_uv), length(vel_uv) > 1e-5);
    let perp_axis = vec2<f32>(-axis.y, axis.x);
    let k = 1.0 + clamp(speed, 0.0, 1.0) * 0.75;   // capped so support stays in 5×5

    // Grid density: low `blob_count` → few big blobs, high → many small ones.
    let dens = clamp((blob_count - 2.0) / 7.0, 0.0, 1.0);
    let cell_scale = mix(2.2, 5.2, dens);
    let gp = uv * cell_scale;
    let gi = floor(gp);

    // Accumulate a metaball field from one wandering blob per cell over the 5×5
    // neighbourhood, using a Wyvill-style finite-support kernel: each blob's weight
    // reaches exactly zero (with zero slope) at radius R, so a blob crossing the
    // neighbourhood edge adds nothing and leaves no seam — unlike an inverse-square
    // sum, which never vanishes and tiles into visible grid lines. Adjacent blobs
    // overlap and their weights add into fused bodies; the velocity stretch above
    // elongates each blob along the pan axis, bridging neighbours into streaks.
    let R = 0.82;
    let inv_r2 = 1.0 / (R * R);
    var field = 0.0;
    for (var dy = -2; dy <= 2; dy = dy + 1) {
        for (var dx = -2; dx <= 2; dx = dx + 1) {
            let cell = gi + vec2<f32>(f32(dx), f32(dy));
            let rnd = hash2(cell + vec2<f32>(1.3, 2.7));
            let wob = vec2<f32>(
                sin(t + rnd.x * 6.2831 + cell.x * 1.3),
                cos(t * 0.87 + rnd.y * 6.2831 + cell.y * 1.7),
            );
            let center = cell + vec2<f32>(0.5) + 0.28 * wob;

            let p = gp - center;
            // Squash the along-axis offset by `k`: the kernel reaches farther in the
            // travel direction, so moving blobs read as elongated and touch sooner.
            let along = dot(p, axis) / k;
            let perp = dot(p, perp_axis);
            let q = (along * along + perp * perp) * inv_r2;
            let w = 1.0 - clamp(q, 0.0, 1.0);
            field = field + w * w * w;   // C² smooth, compact support
        }
    }

    // Surface threshold turns the field into blob mass. `merge` (softness) sets the
    // boundary width: crisp (low) reads as circles cleanly joining into peanuts;
    // soft (high) melts into a lava-lamp haze. Panning lowers the threshold, so the
    // faster you move the more the whole field swells and fuses — motion "connects".
    let edge = mix(0.08, 0.42, clamp((merge - 0.2) / 2.3, 0.0, 1.0));
    let surf = mix(0.50, 0.37, clamp(speed, 0.0, 1.0));
    let dense = smoothstep(surf - edge, surf + edge, field);
    let core = smoothstep(surf + edge, surf + edge + 0.9, field);

    var col = palette(dense, hue);

    // A soft internal glow toward the densest cores, kept low so it reads as heat,
    // not a highlight. Scales with the glow knob.
    col = col + vec3<f32>(0.10, 0.14, 0.20) * pow(core, 1.6) * 0.7 * glow;

    // Barely-there background gradient so the empty field isn't flat black.
    let bg = mix(vec3<f32>(0.012, 0.020, 0.032), vec3<f32>(0.008, 0.012, 0.022), frag.y / res.y);
    col = max(col, bg);

    // Lock-screen ease: settle into a darker, stiller state.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.45 + vec3<f32>(0.004, 0.010, 0.014), l);

    // Optional edge vignette in zoom-independent screen space.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

// Built-in background: "Sunset Birds" — the emptiest of the set, and often the one
// that reads best behind UI. A rich multi-stop sunset gradient, a low sun disc with
// a soft glow, and a few tiny silhouetted birds drifting across on flapping wings.
// Same Push / `@prop` contract as the rest of the built-in set.
//
// Design notes (kept deliberately minimal):
//   * A five-ish-stop vertical gradient: deep dusk blue overhead → violet → rose →
//     amber → gold at the horizon. `warmth` pushes the whole ramp hotter or cooler.
//   * A sun disc low in the sky (`sun_height`) with a broad glow and a faint band of
//     glare along the horizon.
//   * A soft haze layer at the horizon so the sun melts into it.
//   * A handful of distant birds — two-stroke "v" silhouettes — drifting sideways
//     and flapping. The flap is driven as a *discrete sprite-frame animation*
//     (frame = floor(time·fps) % frames), which is exactly the indexing a real
//     animated sprite-sheet atlas will use once the texture slot below is wired.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop sun_height float default=0.5 min=0.0 max=1.0 label="Sun height" group="Sunset"
// @prop warmth float default=0.6 min=0.0 max=1.0 label="Cool → warm" group="Sunset"
// @prop bird_density float default=1.0 min=0.0 max=2.0 label="Bird count" group="Sunset"
// @prop bird_speed float default=1.0 min=0.0 max=4.0 label="Bird drift" group="Sunset"
// @prop haze float default=1.0 min=0.0 max=2.0 label="Horizon haze" group="Sunset"
// @prop vignette float default=0.2 min=0.0 max=1.0 label="Vignette amount" group="Frame"
// @prop vignette_radius float default=1.2 min=0.5 max=2.0 label="Vignette radius" group="Frame"
// @prop vignette_softness float default=0.7 min=0.05 max=2.0 label="Vignette softness" group="Frame"

struct Push {
    res_zoom_time: vec4<f32>,        // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,             // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,           // x = lock_amount, y = alpha
    params: array<vec4<f32>, 4>,     // shader-authored @prop values (16 floats)
};
var<immediate> pc: Push;

// ── Reserved: texture / animated sprite-sheet slot (not yet wired) ────────────
// The renderer drives background shaders with push constants only — no bound
// texture or sampler in the dispatch seam yet (see underwater.wgsl for the full
// note). `params[3]` is reserved as the sprite-sheet control vec4 (zero today):
//   params[3].x = atlas columns        params[3].z = playback fps
//   params[3].y = atlas rows           params[3].w = frame count (0 = cols*rows)
// When a texture arrives, bind it here and sample with `sprite_frame_uv`:
//   @group(0) @binding(0) var atlas_tex: texture_2d<f32>;
//   @group(0) @binding(1) var atlas_smp: sampler;
// This shader already animates its birds on the same discrete-frame clock, so
// swapping the procedural wing for an atlas sample is a drop-in later.
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

// Distance from point `p` to the segment a→b (for the two wing strokes).
fn seg(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// One bird silhouette in local cell space `f` (centred, ~[-0.5,0.5]). `openness`
// (0 = wings down, 1 = wings up) is the current animation frame's wing pose; a
// distant gull is two shallow strokes meeting at the body, tips bent down a touch.
fn bird(f: vec2<f32>, openness: f32, size: f32) -> f32 {
    let p = f / size;
    if (dot(p, p) > 1.6) { return 0.0; }
    let span = 0.9;
    let lift = mix(-0.12, 0.5, openness);                 // wing-tip height by frame
    let tipL = vec2<f32>(-span, lift);
    let tipR = vec2<f32>(span, lift);
    let bend = vec2<f32>(0.0, -0.02);                     // slight body droop
    let d = min(seg(p, bend, tipL), seg(p, bend, tipR));
    // A thin dark silhouette; scale the line width with size so far birds stay crisp.
    return smoothstep(0.16, 0.06, d);
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

    let sun_height = clamp(pc.params[0].x, 0.0, 1.0);
    let warmth = clamp(pc.params[0].y, 0.0, 1.0);
    let bird_density = pc.params[0].z;
    let bird_speed = pc.params[0].w;
    let haze = pc.params[1].x;
    let vignette = pc.params[1].y;
    let vig_radius = pc.params[1].z;
    let vig_softness = pc.params[1].w;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let flow = vec2<f32>(flow_in.x, -flow_in.y);

    // Multi-stop sunset ramp. `h` is 0 at the bottom (horizon), 1 at the top.
    let h = clamp(screen_uv.y * 0.7 + 0.5, 0.0, 1.0);
    let c_horizon = mix(vec3<f32>(1.0, 0.78, 0.30), vec3<f32>(1.0, 0.90, 0.55), warmth);
    let c_low = mix(vec3<f32>(0.98, 0.45, 0.28), vec3<f32>(1.0, 0.55, 0.22), warmth);
    let c_mid = mix(vec3<f32>(0.55, 0.28, 0.42), vec3<f32>(0.80, 0.34, 0.34), warmth);
    let c_high = mix(vec3<f32>(0.14, 0.14, 0.34), vec3<f32>(0.22, 0.16, 0.36), warmth);
    var col = mix(c_horizon, c_low, smoothstep(0.0, 0.28, h));
    col = mix(col, c_mid, smoothstep(0.24, 0.6, h));
    col = mix(col, c_high, smoothstep(0.55, 1.0, h));

    // Sun: low in the sky, warm core and broad glow, plus a horizon glare band.
    let sun_pos = vec2<f32>(0.08, mix(-0.28, 0.30, sun_height));
    let sd = length((screen_uv - sun_pos) * vec2<f32>(1.0, 1.0));
    let disc = smoothstep(0.085, 0.070, sd);
    let glow = pow(smoothstep(1.1, 0.0, sd), 2.4);
    let sun_col = mix(vec3<f32>(1.0, 0.85, 0.55), vec3<f32>(1.0, 0.95, 0.80), warmth);
    col = col + sun_col * (glow * 0.4 + disc * 0.8);
    // Horizon glare: a soft bright band centred on the sun's column.
    let glare = exp(-pow((screen_uv.y - sun_pos.y) * 3.0, 2.0)) * smoothstep(0.9, 0.0, abs(screen_uv.x - sun_pos.x));
    col = col + sun_col * glare * 0.2;

    // Horizon haze so the sun melts into the skyline.
    let hz = exp(-pow((screen_uv.y + 0.15) * 4.5, 2.0));
    col = mix(col, mix(col, sun_col, 0.35), hz * 0.5 * haze);

    // Birds: a sparse scrolling grid high in the sky. Each drifts sideways and flaps
    // on a discrete sprite-frame clock (the same indexing a real atlas would use).
    // Kept above the horizon and thinning toward the top.
    let bp = uv * vec2<f32>(2.2, 4.0)
           + vec2<f32>(time * 0.05 * bird_speed + pan.x * 0.0004 + flow.x * 0.0005, pan.y * 0.0004);
    let bid = floor(bp);
    let bf = fract(bp) - 0.5;
    let hb = hash(bid);
    if (hb > 1.0 - 0.16 * bird_density) {
        // Discrete flap: a 4-frame wing cycle (down → mid → up → mid), advanced by a
        // per-bird fps so the flock doesn't beat in unison.
        let frames = 4.0;
        let fps = 5.0 * (0.6 + 0.9 * fract(hb * 13.0));
        let frame = floor(time * fps + hb * 20.0) % frames;
        let openness = 1.0 - abs(frame - 2.0) * 0.5;        // 0,0.5,1,0.5 over the cycle
        let size = 0.16 + 0.10 * fract(hb * 47.0);
        let m = bird(bf + vec2<f32>(0.0, 0.12 * fract(hb * 91.0)), openness, size);
        // Higher birds sit in front of the sky; fade the flock near the horizon and
        // the very top so they cluster pleasantly around the sun's height.
        let band = smoothstep(-0.5, 0.0, uv.y) * smoothstep(1.1, 0.3, uv.y);
        col = mix(col, col * 0.12, m * band * 0.85);
    }

    // Lock-screen ease: deepen toward night, cool the sky a touch.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.45 + vec3<f32>(0.02, 0.02, 0.05), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

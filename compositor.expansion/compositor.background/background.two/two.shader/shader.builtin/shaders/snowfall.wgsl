// Built-in background: "Snowfall" — a still winter night with snow drifting down
// past a faint line of firs. A cozy companion to the other built-in scenes: same
// quiet, dark, low-contrast mood, same Push/`@prop` contract, so it slots into
// the built-in shader list and the live preview. The falling flakes are the
// star-speck logic, layered for parallax and set gently in motion.
//
// Design notes (kept deliberately restful):
//   * A deep blue vertical gradient — near-black navy at the top easing to a
//     slightly lifted horizon blue toward the ground, with a whisper of slow
//     cloud swell so the sky isn't a flat wash.
//   * Faint conifer silhouettes along the bottom in two depth layers (far bluer
//     and lower, near darker and taller), reading as a distant treeline.
//   * Layered falling snow: four parallax layers of soft round flakes at
//     different sizes and speeds — near flakes are large, bright and fast; far
//     flakes are small, dim and slow — each drifting sideways on a light wind.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop fall_speed float default=1.0 min=0.0 max=4.0 label="Fall speed" group="Snowfall"
// @prop snow_density float default=1.0 min=0.0 max=2.0 label="Snow density" group="Snowfall"
// @prop flake_size float default=1.0 min=0.2 max=2.5 label="Flake size" group="Snowfall"
// @prop wind float default=1.0 min=-2.0 max=2.0 label="Wind" group="Snowfall"
// @prop tree_line float default=1.0 min=0.0 max=2.0 label="Treeline height" group="Snowfall"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Snowfall"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Snowfall"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Snowfall"

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
// out the contract so a sprite-sheet atlas (e.g. detailed snowflake sprites) can
// be dropped in without reworking the shader once the engine gains a texture
// descriptor for pixel programs.
//
// `params[3]` is reserved as the sprite-sheet control vec4 (zero-filled today):
//   params[3].x = atlas columns        params[3].z = playback fps
//   params[3].y = atlas rows           params[3].w = frame count (0 = cols*rows)
//
// When a texture arrives, bind it here and switch the flakes onto the helper:
//   @group(0) @binding(0) var atlas_tex: texture_2d<f32>;
//   @group(0) @binding(1) var atlas_smp: sampler;
//
// Sub-rect UV for the current animation frame of a cols×rows sheet. `cell` is the
// 0..1 coord within one sprite (a flake's local quad remapped to 0..1).
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

// Coverage in [0,1] of a conifer treeline at screen point `px`. Each cell holds
// one fir: a triangle tapering to an apex at a jittered height. Neighbour cells
// are checked so trees overlap into a continuous skyline. `+y = down`, so the
// silhouette fills everything at or below the highest tree edge.
fn treeline(px: vec2<f32>, ground: f32, height: f32, count: f32, seed: f32) -> f32 {
    let s = px.x * count;
    let cell = floor(s);
    var sky = ground;                    // skyline y (smaller = higher)
    for (var k = -1; k <= 1; k = k + 1) {
        let c = cell + f32(k);
        let h = 0.35 + 0.65 * hash(vec2<f32>(c, seed));
        let cx = 0.5 + (hash(vec2<f32>(c, seed + 3.3)) - 0.5) * 0.3;
        let localx = (s - cell) - f32(k);
        let dxn = clamp(abs(localx - cx) / 0.34, 0.0, 1.0);   // 0 apex → 1 edge
        let top_y = ground - height * h * (1.0 - dxn);
        sky = min(sky, top_y);
    }
    return smoothstep(sky - 0.006, sky + 0.006, px.y);
}

// One parallax layer of falling snow added to `col`. Cells scroll downward over
// time (so flakes fall) and drift sideways on the wind; the canvas pan carries
// them by depth. `i` = 1 is the nearest (large, bright, fast) layer.
fn snow_layer(col: vec3<f32>, uv: vec2<f32>, pan: vec2<f32>, time: f32,
              i: i32, density: f32, size: f32, wind: f32, fall: f32) -> vec3<f32> {
    let depth = f32(i);
    let scale = 5.0 * depth;
    let spd = fall * 0.5 / depth;                          // near falls faster
    let wind_off = wind * time * 0.35 / depth + pan.x * 0.001 * depth;
    let sp = vec2<f32>(uv.x * scale + wind_off, uv.y * scale + time * spd + pan.y * 0.001 * depth);
    let id = floor(sp);
    let f = fract(sp) - 0.5;
    let h = hash(id + f32(i) * 31.0);
    if (h > 1.0 - 0.14 * density) {
        let sway = sin(time * 1.0 * fall + h * 40.0) * 0.18;
        let d = length(f - vec2<f32>(sway, 0.0));
        let r = size * 0.13 / depth;                       // near flakes larger
        let flake = smoothstep(r, r * 0.2, d);
        let bright = 0.45 + 0.55 / depth;                  // near flakes brighter
        return col + vec3<f32>(0.85, 0.9, 1.0) * flake * bright;
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

    let fall = pc.params[0].x;
    let snow_density = pc.params[0].y;
    let flake_size = pc.params[0].z;
    let wind = pc.params[0].w;
    let tree_line = pc.params[1].x;
    let vignette = pc.params[1].y;
    let vig_radius = pc.params[1].z;
    let vig_softness = pc.params[1].w;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    // Deep blue night column: near-black navy at the top easing to a slightly
    // lifted horizon blue toward the ground.
    let t = frag.y / res.y;
    var col = mix(vec3<f32>(0.02, 0.035, 0.09), vec3<f32>(0.06, 0.10, 0.19), t);

    // A whisper of slow cloud swell so the sky isn't a flat wash.
    let swell = fbm(uv * 1.1 + pan * 0.0002 + flow * 0.0003 + vec2<f32>(time * 0.006 * fall, time * 0.002));
    col = col + vec3<f32>(0.03, 0.045, 0.07) * pow(swell, 1.6) * 0.4;

    // Far snow drifting behind the trees.
    col = snow_layer(col, uv, pan, time, 4, snow_density, flake_size, wind, fall);
    col = snow_layer(col, uv, pan, time, 3, snow_density, flake_size, wind, fall);

    // Conifer treeline, far → near (near drawn last, on top).
    let tl_far = treeline(screen_uv + vec2<f32>(pan.x * 0.0005, 0.0), 0.46, 0.22 * tree_line, 9.0, 4.0);
    col = mix(col, vec3<f32>(0.03, 0.055, 0.10), tl_far);
    let tl_near = treeline(screen_uv + vec2<f32>(pan.x * 0.0009, 0.0), 0.56, 0.34 * tree_line, 6.0, 19.0);
    col = mix(col, vec3<f32>(0.012, 0.025, 0.05), tl_near);

    // Near snow falling in front of everything.
    col = snow_layer(col, uv, pan, time, 2, snow_density, flake_size, wind, fall);
    col = snow_layer(col, uv, pan, time, 1, snow_density, flake_size, wind, fall);

    // Lock-screen ease: settle the night into a stiller, dimmer hush.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.01, 0.016, 0.03), l);

    // Optional edge vignette in zoom-independent screen space.
    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

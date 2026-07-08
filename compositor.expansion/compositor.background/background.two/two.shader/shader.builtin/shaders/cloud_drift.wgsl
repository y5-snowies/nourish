// Built-in background: "Cloud Drift" — a wide, calm sky of puffy cartoon clouds
// drifting on the wind. Big soft fBm blobs thresholded with a wide smoothstep so
// they stay flat and puffy (never wispy), over a sky that blends from clear day to
// warm sunset. Same quiet mood and the same Push / `@prop` contract as the others.
//
// Design notes (kept deliberately restful):
//   * The sky is a two-stop vertical gradient; the `sky_blend` knob crossfades a
//     daytime palette (blue → pale horizon) into a sunset one (violet → amber), and
//     drops the sun toward the horizon as it does.
//   * A soft sun disc with a broad glow sits off to one side; at sunset it reddens.
//   * Several parallax layers of clouds, each a thresholded fBm field. The threshold
//     (`coverage`) sets how much sky is filled; the smoothstep width (`puffiness`)
//     keeps the edges soft and rounded. Cloud tops catch light, undersides shade.
//   * The wind scrolls the noise slowly; the canvas pan and its flow velocity add a
//     little parallax so scrolling the workspace nudges the clouds.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=4.0 label="Drift speed" group="Sky"
// @prop coverage float default=1.0 min=0.0 max=2.0 label="Cloud cover" group="Sky"
// @prop puffiness float default=1.0 min=0.2 max=2.0 label="Puffiness" group="Sky"
// @prop sky_blend float default=0.35 min=0.0 max=1.0 label="Day → sunset" group="Sky"
// @prop sun_glow float default=1.0 min=0.0 max=2.0 label="Sun glow" group="Sky"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Frame"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Frame"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Frame"

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

// One parallax band of puffy clouds composited over `col`. `depth` fades and slows
// the far layers; `move_` scrolls the field; `tint`/`shade` colour the lit tops and
// shaded undersides for this layer's height in the sky.
fn cloud_layer(col: vec3<f32>, uv: vec2<f32>, move_: vec2<f32>, depth: f32,
               coverage: f32, puff: f32, lit: vec3<f32>, shade: vec3<f32>,
               opacity: f32) -> vec3<f32> {
    let scale = 1.6 / depth;
    let p = uv * scale + move_;
    // Two fBm draws: the shape, and a slightly offset copy to carve lit tops.
    let n = fbm(p);
    let ntop = fbm(p + vec2<f32>(0.0, -0.16));
    // Wide smoothstep threshold → flat, rounded, cartoon-puffy edges. `coverage`
    // slides the threshold (more coverage = lower threshold = more sky filled).
    let thr = 0.62 - 0.14 * coverage;
    let w = 0.16 * puff;
    let mask = smoothstep(thr - w, thr + w, n);
    if (mask <= 0.0) { return col; }
    // Shade by how much taller the top-offset draw is: sunlit crowns, dark bellies.
    let form = smoothstep(thr - w, thr + w * 2.0, ntop);
    let body = mix(shade, lit, clamp(form, 0.0, 1.0));
    return mix(col, body, mask * clamp(opacity, 0.0, 1.0));
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

    let drift = pc.params[0].x;
    let coverage = pc.params[0].y;
    let puff = pc.params[0].z;
    let sky_blend = clamp(pc.params[0].w, 0.0, 1.0);
    let sun_glow = pc.params[1].x;
    let vignette = pc.params[1].y;
    let vig_radius = pc.params[1].z;
    let vig_softness = pc.params[1].w;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let flow = vec2<f32>(flow_in.x, -flow_in.y);

    // Sky gradient: crossfade day → sunset. `h` is 0 at the bottom, 1 at the top.
    let h = clamp(screen_uv.y * 0.75 + 0.5, 0.0, 1.0);
    let day = mix(vec3<f32>(0.78, 0.86, 0.92), vec3<f32>(0.22, 0.48, 0.80), h);
    let dusk = mix(vec3<f32>(0.98, 0.58, 0.30), vec3<f32>(0.14, 0.10, 0.30), pow(h, 0.8));
    var col = mix(day, dusk, sky_blend);

    // Sun: high at midday, sinking to the horizon at sunset; warms as it drops.
    let sun_pos = vec2<f32>(0.34, mix(0.40, -0.04, sky_blend));
    let sd = length((screen_uv - sun_pos) / zoom * vec2<f32>(1.0, 1.0));
    let disc = smoothstep(0.055, 0.045, sd);
    let glow = pow(smoothstep(0.9, 0.0, sd), 2.2);
    let sun_col = mix(vec3<f32>(1.0, 0.97, 0.85), vec3<f32>(1.0, 0.62, 0.32), sky_blend);
    col = col + sun_col * (glow * 0.35 + disc * 0.9) * sun_glow;

    // Cloud tints follow the sky: bright white by day, peach-lit at sunset.
    let lit = mix(vec3<f32>(0.98, 0.99, 1.0), vec3<f32>(1.0, 0.80, 0.60), sky_blend);
    let shade = mix(vec3<f32>(0.62, 0.68, 0.78), vec3<f32>(0.42, 0.30, 0.42), sky_blend);

    // Wind carries clouds sideways; pan/flow add parallax. Far layers move slower.
    let wind = time * 0.02 * drift;
    // Far → near, near drawn last (on top).
    col = cloud_layer(col, uv, vec2<f32>(wind * 0.35, 0.0) + pan * 0.00016 + flow * 0.00020, 2.6,
                      coverage, puff, lit * 0.94, shade, 0.85);
    col = cloud_layer(col, uv, vec2<f32>(wind * 0.6, 0.02) + pan * 0.00028 + flow * 0.00034, 1.7,
                      coverage, puff, lit * 0.97, shade, 0.92);
    col = cloud_layer(col, uv, vec2<f32>(wind, -0.01) + pan * 0.00044 + flow * 0.00052, 1.0,
                      coverage, puff, lit, shade, 1.0);

    // Lock-screen ease: settle the sky toward a deeper dusk.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.55 + vec3<f32>(0.03, 0.02, 0.05), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

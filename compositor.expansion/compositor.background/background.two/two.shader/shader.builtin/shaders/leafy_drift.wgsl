// Built-in background: "Leafy Drift" — the *inside* companion to Leafy Galaxy.
// You're within the leafy world's air: a calm green light with an array of leaves
// drifting on the wind, scattered like the star field and carried by the canvas —
// panning gusts them along. Same quiet mood, same Push/`@prop` contract.
//
// Design notes (kept deliberately restful):
//   * A soft green gradient with a gentle light from one side — never busy.
//   * Several parallax layers of leaf sprites, seeded per grid cell (density knob),
//     each aligned to the wind, fluttering, and tumbling slowly.
//   * The wind carries them diagonally over time; the canvas pan and its velocity
//     (flow) push and tilt them, so scrolling the workspace stirs the leaves.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop wind float default=1.0 min=0.0 max=6.0 label="Wind speed" group="Drift"
// @prop leaf_density float default=1.0 min=0.0 max=2.0 label="Leaf density" group="Drift"
// @prop lushness float default=1.0 min=0.0 max=2.0 label="Lushness" group="Drift"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Drift"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Drift"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Drift"

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

// A leaf silhouette in local space: points along ±y, width tapering to the tips,
// a touch teardrop-heavy at the base, plus a faint central vein. Returns 0..1.
fn leaf_mask(p: vec2<f32>) -> f32 {
    if (abs(p.y) > 1.0) { return 0.0; }
    let y = p.y;
    let halfw = 0.44 * (1.0 - y * y) * (1.0 - 0.28 * y);   // slight teardrop
    let m = smoothstep(0.035, -0.01, abs(p.x) - halfw);
    // A darker midrib is applied by the caller via this returned crease factor.
    return m;
}

// One parallax layer of wind-blown leaves. `flow` is the pan-velocity (flow_offset)
// that gusts and tilts the leaves; `pan` translates them by depth.
fn leaf_layer(col: vec3<f32>, uv: vec2<f32>, pan: vec2<f32>, flow: vec2<f32>,
              time: f32, i: i32, wind: f32, density: f32, lush: f32) -> vec3<f32> {
    let depth = 1.0 + f32(i) * 0.7;
    let gust = 1.0 + 0.4 * sin(time * 0.3 + f32(i) * 1.7);
    // Wind carries leaves down-and-across; the pan velocity adds to it.
    let wdir = vec2<f32>(0.9, -0.45);
    let wvel = wind * gust * 0.05 / depth;
    let move_ = wdir * time * wvel
              + pan * (0.0013 * depth)
              + flow * (0.0016 * depth);
    let scale = 6.0 / depth;
    let coord = uv * scale + move_;
    let c = floor(coord);
    let f = fract(coord) - 0.5;
    let h = hash(c + f32(i) * 41.0);
    if (h <= 1.0 - 0.09 * density) { return col; }

    let ph = fract(h * 97.0);
    // Orientation: aligned to the wind, plus a slow flutter and a tilt from the
    // pan velocity (a gust cants every leaf the same way).
    let flutter = sin(time * 1.6 * gust + h * 25.0) * 0.7;
    let tilt = clamp((flow.x - flow.y) * 0.0008, -0.8, 0.8);
    let ang = atan2(wdir.y, wdir.x) + flutter + tilt + ph * 6.2832;
    let ca = cos(-ang);
    let sa = sin(-ang);
    let size = 0.30 + 0.16 * ph;
    let lp = vec2<f32>(ca * f.x - sa * f.y, sa * f.x + ca * f.y) / size;
    let m = leaf_mask(lp);
    if (m <= 0.0) { return col; }

    // Colour: mostly greens with a few golden/amber leaves for life. A midrib and
    // a soft top-lit sheen give each leaf a little form.
    var lc = mix(vec3<f32>(0.10, 0.26, 0.11), vec3<f32>(0.26, 0.34, 0.12), ph) * (0.7 + 0.5 * lush);
    if (ph > 0.82) { lc = mix(lc, vec3<f32>(0.36, 0.24, 0.07), 0.7); }   // autumn accent
    lc = lc * (0.75 + 0.35 * smoothstep(-0.6, 0.6, lp.y));               // top-lit sheen
    lc = lc * (1.0 - 0.35 * smoothstep(0.06, 0.0, abs(lp.x)));           // midrib
    return mix(col, lc, m * (0.95 / depth));
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

    let wind = pc.params[0].x;
    let density = pc.params[0].y;
    let lush = pc.params[0].z;
    let vignette = pc.params[0].w;
    let vig_radius = pc.params[1].x;
    let vig_softness = pc.params[1].y;

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    let pan = vec2<f32>(pan_in.x, -pan_in.y);
    let flow = vec2<f32>(flow_in.x, -flow_in.y);

    // Soft green air with a gentle light from the upper-left (sun through leaves).
    var col = mix(vec3<f32>(0.03, 0.075, 0.045), vec3<f32>(0.06, 0.14, 0.08), clamp(screen_uv.y * 0.5 + 0.5, 0.0, 1.0));
    col = col + vec3<f32>(0.05, 0.10, 0.05) * exp(-((screen_uv.x + 0.5) * (screen_uv.x + 0.5) + (screen_uv.y - 0.4) * (screen_uv.y - 0.4)) * 1.4) * (0.6 + 0.4 * lush);
    // A very soft canopy dapple so the empty air isn't flat.
    col = col + vec3<f32>(0.03, 0.06, 0.03) * fbm(uv * 1.4 + pan * 0.0003 + vec2<f32>(time * 0.02 * wind, 0.0)) * 0.4;

    // Leaves, far → near (near layer drawn last, on top).
    col = leaf_layer(col, uv, pan, flow, time, 3, wind, density, lush);
    col = leaf_layer(col, uv, pan, flow, time, 2, wind, density, lush);
    col = leaf_layer(col, uv, pan, flow, time, 1, wind, density, lush);
    col = leaf_layer(col, uv, pan, flow, time, 0, wind, density, lush);

    // Lock-screen ease: still the air and deepen the green toward dusk.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.006, 0.016, 0.01), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    return vec4<f32>(col, 1.0) * (alpha * 0.75);
}

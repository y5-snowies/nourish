// TB — `textures`: an animated SPRITE SHEET, orbiting every window.
//
// This is the input a shader cannot compute. Everything else the engine offers is
// either generated (noise, gradients, SDFs) or is the desktop itself; a texture is
// where hand-drawn or photographic detail comes from. `sparks.png` is a 4x4 atlas
// of one spark igniting, burning and dying — sixteen frames of art that no amount
// of arithmetic in here would produce.
//
// WHAT TO LOOK FOR
// ----------------
// Sparks circle every window, each one playing the sixteen-frame burn on its own
// phase, so they do not flash in unison. Open a second window and it gets its own
// set. Move a window and the sparks follow it — they are positioned from the live
// window rect, not from anything baked.
//
// Turn `frames` down to 1: the animation freezes on whatever frame each spark
// happens to be on, and you can see the individual atlas cells. Turn `size` up to
// see one cell filling a large area — the art holds because it is art, not a
// procedural blob.
//
// HOW A TEXTURE IS WIRED
// ----------------------
// It is not a new binding kind. `pipeline.json` declares a name under `textures`,
// a pass names it in `inputs` exactly as it would name a target, and it arrives at
// `@group(0) @binding(1 + n)` in the same sorted-by-binding-name order as every
// other input. Nothing in this file could tell you whether `sparks` is a PNG on
// disk or an intermediate the graph rendered.
//
// TWO THINGS THAT WILL BITE
// -------------------------
// 1. The sampler clamps at the edge of the WHOLE atlas, not at each cell. Sampling
//    a cell right up to its border lets bilinear filtering pull in the neighbour,
//    which shows as a faint ghost of the next frame. `cell_uv` insets by half a
//    texel — that is what `textureDimensions` is for here, and it is also how a
//    shader learns a texture's size, since the push does not carry it.
//
// 2. A sampled texel is STRAIGHT alpha, as authored. Nothing premultiplies on the
//    way in. So compositing is `rgb * a`, done here, in linear space, where it is
//    correct.
//
// @prop count   float default=7.0  min=1.0  max=16.0 step=1.0   label="Sparks per window" group="Sparks"
// @prop size    float default=0.075 min=0.02 max=0.30 step=0.005 label="Spark size"       group="Sparks"
// @prop orbit   float default=0.55 min=0.00 max=1.50 step=0.01  label="Orbit spread"      group="Sparks"
// @prop speed   float default=0.35 min=0.00 max=2.00 step=0.01  label="Orbit speed"       group="Sparks"
// @prop frames  float default=14.0 min=1.0  max=40.0 step=1.0   label="Animation rate"    group="Sparks"
// @prop glow    float default=1.20 min=0.00 max=3.00 step=0.05  label="Spark brightness"  group="Sparks"

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
// The declared texture. A target would be declared with exactly this line.
@group(0) @binding(1) var sparks: texture_2d<f32>;

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,
    srcs: array<vec4<f32>, 256>,
    attrs: array<vec4<f32>, 256>,
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;

struct Times {
    count: u32,
    _pad: vec3<u32>,
    // x = opened, y = entered screen, z = left screen, w = reserved
    life: array<vec4<f32>, 256>,
    state: array<vec4<f32>, 256>,
    drag: array<vec4<f32>, 256>,
};
@group(1) @binding(2) var<uniform> times: Times;

// The atlas layout. Cells rather than pixels, so the same shader works if the
// sheet is redrawn at a different resolution — only a change in the GRID would
// need editing here.
const COLS: f32 = 4.0;
const ROWS: f32 = 4.0;

/// UV of `local` (0..1 within one cell) in cell `frame` of the atlas.
///
/// The half-texel inset is not cosmetic: without it the filter reaches past the
/// cell edge into the next frame of the animation, and a spark shows a faint
/// double image that is very hard to attribute to sampling.
fn cell_uv(local: vec2<f32>, frame: f32) -> vec2<f32> {
    let dims = vec2<f32>(textureDimensions(sparks));
    let grid = vec2<f32>(COLS, ROWS);
    let f = clamp(frame, 0.0, COLS * ROWS - 1.0);
    let cell = vec2<f32>(f % COLS, floor(f / COLS));
    let inset = 0.5 / dims * grid;
    let inner = clamp(local, inset, vec2<f32>(1.0) - inset);
    return (cell + inner) / grid;
}

fn hash(n: f32) -> f32 {
    return fract(sin(n * 127.1) * 43758.5453);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let aspect = res.x / res.y;
    let zoom = max(pc.res_zoom_time.z, 0.0001);

    let count = pc.params[0].x;
    let size = pc.params[0].y;
    let orbit = pc.params[0].z;
    let speed = pc.params[0].w;
    let frames = pc.params[1].x;
    let glow = pc.params[1].y;

    var col = mix(
        vec3<f32>(0.020, 0.016, 0.028),
        vec3<f32>(0.055, 0.030, 0.035),
        smoothstep(0.0, 1.0, uv.y),
    );

    let n = min(windows.count, 256u);

    // The band, back to front. Windows first so the sparks land over them.
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let hi = r.xy + r.zw;
        if (uv.x < r.x || uv.x > hi.x || uv.y < r.y || uv.y > hi.y) {
            continue;
        }
        let s = windows.srcs[i];
        let local = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));
        let pp = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
        let a = clamp(pp.a, 0.0, 1.0) * windows.attrs[i].y;
        col = col * (1.0 - a) + pp.rgb * a;
    }

    // Sparks. Additive, so overlapping ones build rather than occlude — which is
    // what fire does and what alpha compositing would get wrong.
    var fire = vec3<f32>(0.0);
    let per = u32(clamp(count, 1.0, 16.0));
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let mid = r.xy + r.zw * 0.5;
        let half = r.zw * 0.5;
        // Age, so a window that has just opened is not instantly ringed with
        // fire. `life.x` is when it opened, on the same clock as `t`.
        let age = clamp((t - times.life[i].x) * 1.5, 0.0, 1.0);
        if (age <= 0.0) {
            continue;
        }
        for (var k = 0u; k < per; k = k + 1u) {
            let seed = hash(f32(i) * 7.31 + f32(k) * 3.77);
            let ang = seed * 6.2831853 + t * speed * (0.6 + seed);
            // Around the window's own box, pushed out by `orbit`.
            let ring = half * (1.0 + orbit * (0.35 + 0.65 * seed));
            let at = mid + vec2<f32>(cos(ang), sin(ang)) * ring;

            // Sprite footprint, kept square on screen by the aspect correction.
            // WORLD-SCALED, not screen-scaled. `size` is a world length, and the
            // `* zoom` is what keeps a spark the same size RELATIVE TO THE WINDOW
            // it orbits at every zoom level. Without it the ring still follows the
            // window — its radius comes from the rect, which the engine already
            // transformed — but each spark stays a fixed fraction of the SCREEN, so
            // zooming out shrinks the window and not the sparks and they swell up
            // around it. Positions are handled for you; lengths are not.
            // See SKILL.md 3.1.
            let sp = size * zoom * (0.6 + 0.8 * seed);
            let extent = vec2<f32>(sp / aspect, sp);
            let local = (uv - at) / extent + vec2<f32>(0.5);
            if (local.x < 0.0 || local.x > 1.0 || local.y < 0.0 || local.y > 1.0) {
                continue;
            }

            // Each spark on its own phase, so sixteen frames do not flash in
            // unison across the ring.
            let frame = floor(t * frames + seed * COLS * ROWS) % (COLS * ROWS);
            let texel = textureSampleLevel(sparks, samp, cell_uv(local, frame), 0.0);
            // STRAIGHT alpha in, so the multiply is ours to do.
            fire = fire + texel.rgb * texel.a * glow * age;
        }
    }
    col = col + fire;

    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

// TB — `textures`, THE COVERING EXAMPLE. Pass 1 of 2 (before-content).
//
// Every facility the texture surface has, in one bundle, so there is one place to
// read rather than three. The three `tb-texture-*` siblings each isolate one idiom;
// this is the whole thing working together, and it is the one to copy from.
//
// WHAT IS BEING EXERCISED
// -----------------------
//   * THREE textures in one bundle, with different colour spaces.
//   * A texture as ART      — `sparks.png`, an sRGB 4x4 sprite atlas (this pass).
//   * A texture as a MATERIAL — `paper.png`, a LINEAR height + slope field (this pass).
//   * A texture as DATA     — `grade.png`, a LINEAR colour LUT (pass 2).
//   * Textures and engine built-ins MIXED in one `inputs` map, so the sorted
//     binding order is visible (pass 2: `lut`, `scene`, `sparks`).
//   * ONE texture read by TWO passes, in two different BANDS — `sparks` is here
//     and in pass 2. A texture is bundle-level; nothing is per pass.
//   * Alongside `window_geometry` / `window_textures` / `window_times` /
//     `pointer_state`, because textures compose with everything else.
//
// USE THE `Show` DROPDOWN. It isolates each facility so you can see what each one
// contributes, and — more useful when something is wrong — what the frame looks
// like with that one part switched off.
//
// THE `Show` PROP IS DECLARED IN BOTH PASSES, ON PURPOSE
// -----------------------------------------------------
// Props union across the bundle BY NAME, so the same `@prop show` declared in two
// passes is one variable with one slider, and both passes read the same live
// value. That is how a bundle-wide mode works. It is not a duplicate.
//
// @prop show    int   default=0 choices="Everything,Material only,Sprites only,Grade only,No textures" label="Show" group="Debug"
// @prop tiles   float default=6.0  min=1.0  max=24.0 step=1.0   label="Tiles across"      group="Material"
// @prop depth   float default=1.00 min=0.0  max=3.0  step=0.05  label="Grain depth"       group="Material"
// @prop count   float default=6.0  min=1.0  max=16.0 step=1.0   label="Sparks per window" group="Sprites"
// @prop size    float default=0.070 min=0.02 max=0.30 step=0.005 label="Spark size"       group="Sprites"
// @prop speed   float default=0.35 min=0.00 max=2.00 step=0.01  label="Orbit speed"       group="Sprites"
// @prop frames  float default=14.0 min=1.0  max=40.0 step=1.0   label="Animation rate"    group="Sprites"
// @prop glow    float default=1.10 min=0.00 max=3.00 step=0.05  label="Spark brightness"  group="Sprites"

// COORDINATE CHOICE — BOTH ANSWERS, IN ONE PASS
// ---------------------------------------------
// The engine transforms the POSITIONS it hands you (`windows.rects`,
// `pointer.at`) but never the LENGTHS you write yourself. So every magnitude in a
// shader is a screen length until you make it otherwise, and one left in screen
// units grows relative to the desktop as the user zooms out. Zoom the world out
// and watch: anything that does not shrink with it is in the wrong space.
//
//   * The SPARKS are world lengths — `size * zoom` — so each stays the same size
//     relative to the window it orbits at any zoom.
//   * The PAPER is a screen length — no `zoom` — because it is a sheet over the
//     display, the same call a vignette or film grain makes.
//
// Both are correct; what is not correct is failing to decide. See SKILL.md 3.1.

enable wgpu_binding_array;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
// Sorted by BINDING NAME — `paper` before `sparks`. Textures take their place in
// the same list as targets and built-ins; there is no separate group for art.
@group(0) @binding(1) var paper: texture_2d<f32>;
@group(0) @binding(2) var sparks: texture_2d<f32>;

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

struct Pointer {
    // xy = screen UV, z = held buttons, w = reserved
    at: vec4<f32>,
    // x = last pressed, y = last released, z / w = reserved
    moment: vec4<f32>,
};
@group(1) @binding(3) var<uniform> pointer: Pointer;

const BTN_LEFT: u32 = 1u;

// `Show` values, matching the `choices=` order above.
const SHOW_ALL: i32 = 0;
const SHOW_MATERIAL: i32 = 1;
const SHOW_SPRITES: i32 = 2;
const SHOW_GRADE: i32 = 3;
const SHOW_NONE: i32 = 4;

// The sprite atlas layout. Cells, not pixels, so redrawing the sheet at another
// resolution needs no edit here — only a change of GRID would.
const COLS: f32 = 4.0;
const ROWS: f32 = 4.0;

/// UV of `local` (0..1 within one cell) in cell `frame`.
///
/// THE ATLAS TRAP: the sampler clamps at the edge of the WHOLE image, not per
/// cell, so sampling a cell right to its border lets bilinear filtering pull in
/// the neighbouring frame — a faint double image that is very hard to attribute.
/// The half-texel inset is the fix, and `textureDimensions` is how the shader
/// learns the size, since the push does not carry it.
fn cell_uv(local: vec2<f32>, frame: f32) -> vec2<f32> {
    let dims = vec2<f32>(textureDimensions(sparks));
    let grid = vec2<f32>(COLS, ROWS);
    let f = clamp(frame, 0.0, COLS * ROWS - 1.0);
    let cell = vec2<f32>(f % COLS, floor(f / COLS));
    let inset = 0.5 / dims * grid;
    return (cell + clamp(local, inset, vec2<f32>(1.0) - inset)) / grid;
}

fn hash(n: f32) -> f32 {
    return fract(sin(n * 127.1) * 43758.5453);
}

/// The paper material: a tiling surface lit from where the hand is.
///
/// THE TILING TRAP: there is one sampler and it CLAMPS, so the repeat is `fract()`
/// here. That is legal only because the image is authored seamless. And it must be
/// `textureSampleLevel(..., 0.0)`: `fract` jumps at every tile edge, and an
/// automatic-LOD sample reads that jump as a huge derivative and bands along every
/// seam.
fn material(uv: vec2<f32>, aspect: f32, tiles: f32, depth: f32) -> vec3<f32> {
    let tiled = fract(vec2<f32>(uv.x * aspect, uv.y) * tiles);
    let m = textureSampleLevel(paper, samp, tiled, 0.0);

    // LINEAR data: R is a height, G/B are signed slopes biased to 0.5. Declared
    // `"srgb": false` because these are numbers, not colours — linearising them
    // would bend the relief while still rendering something plausible.
    let height = m.r;
    let slope = (m.gb - vec2<f32>(0.5)) * 2.0 * depth;
    let normal = normalize(vec3<f32>(-slope.x, -slope.y, 1.0));

    let held = (u32(pointer.at.z) & BTN_LEFT) != 0u;
    var power = 1.0;
    var falloff = 0.55;
    if (held) {
        power = 1.6;
        falloff = 0.33;
    }
    let to_light = vec2<f32>((pointer.at.x - uv.x) * aspect, pointer.at.y - uv.y);
    let l = normalize(vec3<f32>(to_light, falloff * 0.75));
    let lambert = max(dot(normal, l), 0.0);
    let fall = power / (1.0 + pow(length(to_light) / falloff, 2.0));

    let tone = vec3<f32>(0.68, 0.66, 0.61) * (0.55 + 0.45 * height);
    return tone * (0.10 + lambert * fall) * vec3<f32>(1.00, 0.90, 0.76) + tone * 0.04;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let aspect = res.x / res.y;
    let zoom = max(pc.res_zoom_time.z, 0.0001);

    let show = i32(pc.params[0].x + 0.5);
    let tiles = pc.params[0].y;
    let depth = pc.params[0].z;
    let count = pc.params[0].w;
    let size = pc.params[1].x;
    let speed = pc.params[1].y;
    let frames = pc.params[1].z;
    let glow = pc.params[1].w;

    // The material carries the backdrop unless it is the thing being switched off.
    let want_material = show == SHOW_ALL || show == SHOW_MATERIAL;
    var col = vec3<f32>(0.055, 0.052, 0.050);
    if (want_material) {
        col = material(uv, aspect, tiles, depth);
    }

    let n = min(windows.count, 256u);

    // The band, back to front. This bundle owns it (`windows: "world"`), so the
    // sprites can land OVER the windows rather than only around them.
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

    // Sprites: sRGB ART. Additive, so overlapping sparks build rather than
    // occlude — which is what fire does and what alpha compositing gets wrong.
    let want_sprites = show == SHOW_ALL || show == SHOW_SPRITES;
    if (want_sprites) {
        var fire = vec3<f32>(0.0);
        let per = u32(clamp(count, 1.0, 16.0));
        for (var i = 0u; i < n; i = i + 1u) {
            let r = windows.rects[i];
            let mid = r.xy + r.zw * 0.5;
            let half = r.zw * 0.5;
            // A window that just opened is not instantly ringed with fire.
            // `life.x` is when it opened, on the same clock as `t`.
            let age = clamp((t - times.life[i].x) * 1.5, 0.0, 1.0);
            if (age <= 0.0) {
                continue;
            }
            for (var k = 0u; k < per; k = k + 1u) {
                let seed = hash(f32(i) * 7.31 + f32(k) * 3.77);
                let ang = seed * 6.2831853 + t * speed * (0.6 + seed);
                let at = mid + vec2<f32>(cos(ang), sin(ang)) * half * (1.0 + 0.5 * seed);

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
                // unison around the ring.
                let frame = floor(t * frames + seed * COLS * ROWS) % (COLS * ROWS);
                let texel = textureSampleLevel(sparks, samp, cell_uv(local, frame), 0.0);
                // STRAIGHT alpha in. Nothing premultiplied on the way to the GPU,
                // so the multiply is ours, here, in linear space.
                fire = fire + texel.rgb * texel.a * glow * age;
            }
        }
        col = col + fire;
    }

    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

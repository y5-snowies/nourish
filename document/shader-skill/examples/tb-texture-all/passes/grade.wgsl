// TB — `textures`, THE COVERING EXAMPLE. Pass 2 of 2 (AFTER-CONTENT).
//
// Two things this pass exists to show that pass 1 cannot:
//
// 1. A TEXTURE AND AN ENGINE BUILT-IN IN ONE `inputs` MAP. This pass reads
//    `{"lut": "grade", "scene": "content", "sparks": "sparks"}` — one texture, one
//    engine image, one texture again. They share ONE namespace and ONE binding
//    list, sorted by binding name, so the bindings below are `lut` = 1,
//    `scene` = 2, `sparks` = 3. Read the order off the sort; do not assume the
//    order you wrote.
//
// 2. THE SAME TEXTURE IN TWO PASSES, IN TWO BANDS. `sparks` is bound here and in
//    pass 1, which runs before content. A texture is declared once for the bundle
//    and any pass may name it; there is no per-pass texture list and no cost to
//    the second use beyond the descriptor.
//
// And the LUT itself is the third thing a texture can be: DATA. `grade.png` is a
// 16x16x16 colour cube laid out as sixteen 16x16 slices side by side — 4096
// measured substitutions that no expression reproduces, because a grade is
// authored by eye.
//
// `"srgb": false` IS LOAD-BEARING HERE
// -----------------------------------
// Those bytes are coordinates, not colours. Declared `true` (the default) the
// hardware would linearise every one on the way in — right for artwork, ruinous
// for a table, because entry (0.5, 0.5, 0.5) would stop meaning "mid grey maps
// here". The picture still renders. It is just wrong, in a way that reads as a bad
// grade rather than as a bug. That is exactly why the flag is per texture and is
// never guessed. Flip it in `pipeline.json` and reload to see it.
//
// WHY AFTER-CONTENT: a grade is about the finished image. Run before content it
// would grade the background and leave every window untouched, which is not a
// grade, it is a wallpaper tint.
//
// `show` is declared in pass 1 as well. Props union BY NAME, so this is the same
// slider and the same live value, not a second one.
//
// @prop show     int   default=0 choices="Everything,Material only,Sprites only,Grade only,No textures" label="Show" group="Debug"
// @prop amount   float default=1.00 min=0.0 max=1.0 step=0.01 label="Grade strength" group="Grade"
// @prop exposure float default=1.00 min=0.5 max=1.8 step=0.01 label="Exposure"       group="Grade"
// @prop sparkle  float default=0.90 min=0.0 max=3.0 step=0.05 label="Cursor spark"   group="Sprites"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
// SORTED BY BINDING NAME: lut, scene, sparks. Two textures with an engine image
// between them, and nothing in these declarations says which is which.
@group(0) @binding(1) var lut: texture_2d<f32>;
@group(0) @binding(2) var scene: texture_2d<f32>;
@group(0) @binding(3) var sparks: texture_2d<f32>;

struct Pointer {
    at: vec4<f32>,
    moment: vec4<f32>,
};
@group(1) @binding(3) var<uniform> pointer: Pointer;

const SHOW_ALL: i32 = 0;
const SHOW_SPRITES: i32 = 2;
const SHOW_GRADE: i32 = 3;

const N: f32 = 16.0;
const COLS: f32 = 4.0;
const ROWS: f32 = 4.0;

/// Look `c` up in the strip, interpolating between the two nearest blue slices.
///
/// The half-texel offsets keep entry k reading the CENTRE of texel k rather than
/// its corner. Without them the whole table is biased by half a cell and the
/// grade drifts, most visibly in the neutrals.
fn lookup(c: vec3<f32>) -> vec3<f32> {
    let rgb = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let b = rgb.b * (N - 1.0);
    let z0 = floor(b);
    let z1 = min(z0 + 1.0, N - 1.0);
    let fz = b - z0;
    let x = (rgb.r * (N - 1.0) + 0.5) / (N * N);
    let y = (rgb.g * (N - 1.0) + 0.5) / N;
    let a = textureSampleLevel(lut, samp, vec2<f32>(x + z0 / N, y), 0.0).rgb;
    let d = textureSampleLevel(lut, samp, vec2<f32>(x + z1 / N, y), 0.0).rgb;
    return mix(a, d, fz);
}

fn cell_uv(local: vec2<f32>, frame: f32) -> vec2<f32> {
    let dims = vec2<f32>(textureDimensions(sparks));
    let grid = vec2<f32>(COLS, ROWS);
    let f = clamp(frame, 0.0, COLS * ROWS - 1.0);
    let cell = vec2<f32>(f % COLS, floor(f / COLS));
    let inset = 0.5 / dims * grid;
    return (cell + clamp(local, inset, vec2<f32>(1.0) - inset)) / grid;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let aspect = res.x / res.y;

    let show = i32(pc.params[0].x + 0.5);
    let amount = pc.params[0].y;
    let exposure = pc.params[0].z;
    let sparkle = pc.params[0].w;

    var col = textureSampleLevel(scene, samp, uv, 0.0).rgb * exposure;

    if (show == SHOW_ALL || show == SHOW_GRADE) {
        col = mix(col, lookup(col), clamp(amount, 0.0, 1.0));
    }

    // The same atlas again, over the graded image, at the pointer — one sprite,
    // so the "one texture, two passes, two bands" claim is visible rather than
    // merely stated.
    if (show == SHOW_ALL || show == SHOW_SPRITES) {
        let sp = 0.055;
        let extent = vec2<f32>(sp / aspect, sp);
        let local = (uv - pointer.at.xy) / extent + vec2<f32>(0.5);
        if (local.x >= 0.0 && local.x <= 1.0 && local.y >= 0.0 && local.y <= 1.0) {
            let frame = floor(t * 18.0) % (COLS * ROWS);
            let texel = textureSampleLevel(sparks, samp, cell_uv(local, frame), 0.0);
            col = col + texel.rgb * texel.a * sparkle;
        }
    }

    return vec4<f32>(col, 1.0);
}

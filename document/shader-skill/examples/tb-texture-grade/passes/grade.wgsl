// TB — `textures` as DATA: a colour-grading LUT over the whole desktop.
//
// The other half of what textures are for. `tb-texture-sheet` samples art; this
// samples a lookup table — 4096 measured colour substitutions that no expression
// reproduces, because the whole point of a grade is that it was authored by eye
// rather than derived. `grade.png` is a 16x16x16 cube laid out as sixteen 16x16
// slices side by side, which is the standard strip layout every colour tool
// exports.
//
// `"srgb": false` IS THE POINT OF THIS EXAMPLE
// -------------------------------------------
// These bytes are coordinates, not colours. Declared `srgb: true` (the default)
// the hardware would linearise every one of them on the way in — the same
// transform that is right for artwork and destroys a table, because entry
// (0.5, 0.5, 0.5) would stop meaning "mid grey maps here" and start meaning
// something around 0.21. The picture would still render, and would be wrong in a
// way that looks like a bad grade rather than like a bug. That is exactly why the
// flag is declared per texture and never guessed.
//
// Flip it to `true` in `pipeline.json` and reload if you want to see it: the
// image goes muddy and the shadows crush.
//
// WHAT TO LOOK FOR
// ----------------
// A cool teal lift in the shadows, a warm roll-off in the highlights, a little
// more saturation — over everything, windows included. Slide `amount` to 0 for
// the untouched desktop and back up; that A/B is the whole test.
//
// WHY AFTER-CONTENT
// -----------------
// A grade is about the finished image. Running before content would grade the
// background and leave every window ungraded, which is not a grade, it is a
// wallpaper tint. `when: "after-content"` plus `requires: ["composited_scene"]`
// buys the composited desktop as `scene`.
//
// @prop amount   float default=1.00 min=0.0 max=1.0 step=0.01 label="Grade strength" group="Grade"
// @prop exposure float default=1.00 min=0.5 max=1.8 step=0.01 label="Exposure"       group="Grade"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
// Sorted by BINDING NAME, so `lut` comes before `scene` — the same rule that
// orders target inputs. A texture gets no special place in the order.
@group(0) @binding(1) var lut: texture_2d<f32>;
@group(0) @binding(2) var scene: texture_2d<f32>;

// Cube edge. Sixteen is the usual size and is what `make-texture-assets.py`
// writes; it is stated here rather than derived from `textureDimensions` because
// a strip of N slices of NxN is NxN wide either way and the two cannot be told
// apart from the dimensions alone.
const N: f32 = 16.0;

/// Look `c` up in the strip, interpolating between the two nearest blue slices.
///
/// The half-texel offsets are what keep entry k reading the CENTRE of texel k
/// rather than its corner; without them the whole table is biased by half a cell
/// and the grade drifts, most visibly in the neutrals.
fn lookup(c: vec3<f32>) -> vec3<f32> {
    let rgb = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let b = rgb.b * (N - 1.0);
    let z0 = floor(b);
    let z1 = min(z0 + 1.0, N - 1.0);
    let fz = b - z0;

    // Position within one slice.
    let x = (rgb.r * (N - 1.0) + 0.5) / (N * N);
    let y = (rgb.g * (N - 1.0) + 0.5) / N;

    let a = textureSampleLevel(lut, samp, vec2<f32>(x + z0 / N, y), 0.0).rgb;
    let d = textureSampleLevel(lut, samp, vec2<f32>(x + z1 / N, y), 0.0).rgb;
    return mix(a, d, fz);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let amount = pc.params[0].x;
    let exposure = pc.params[0].y;

    let src = textureSampleLevel(scene, samp, uv, 0.0).rgb * exposure;
    let graded = lookup(src);
    return vec4<f32>(mix(src, graded, clamp(amount, 0.0, 1.0)), 1.0);
}

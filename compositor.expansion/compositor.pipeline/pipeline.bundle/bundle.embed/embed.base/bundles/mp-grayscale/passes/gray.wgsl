// "mp-grayscale" — DESATURATE THE WHOLE DESKTOP.
//
// The smallest useful after-content bundle there is: one input, one line of
// maths. Worth shipping as the thing to copy when starting a new post-effect —
// everything structural is here and nothing else is.
//
// `weights` picks how the grey is measured, and the two answers differ visibly:
// Rec.709 luma matches how bright a colour LOOKS (green counts for ~72%, blue for
// ~7%), while a flat average counts the channels equally and turns saturated blues
// into mid-grey rather than near-black. Luma is right for "make this monochrome";
// average is right for "show me the raw channel content".
//
// @prop amount  float default=1.0 min=0.0 max=1.0 step=0.01 label="Amount"  group="Grayscale"
// @prop weights int   default=0 choices="Rec.709 luma,Flat average"  label="Measure" group="Grayscale"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let src = textureSampleLevel(scene, samp, frag.xy / res, 0.0).rgb;
    let amount = clamp(pc.params[0].x, 0.0, 1.0);

    var w = vec3<f32>(0.2126, 0.7152, 0.0722);
    if (i32(round(pc.params[0].y)) == 1) {
        w = vec3<f32>(1.0 / 3.0);
    }
    var col = mix(src, vec3<f32>(dot(src, w)), amount);
    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

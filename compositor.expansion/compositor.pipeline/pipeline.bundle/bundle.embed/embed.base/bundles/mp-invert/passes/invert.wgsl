// "mp-invert" — WHITE ↔ BLACK.
//
// Two ways to do it, and the difference is the whole demonstration.
//
// `Full` is the classic `1 - rgb`: it inverts lightness AND hue, so a blue window
// chrome comes back orange. That is what "invert" means in most tools, and it is
// what you want when the goal is a hard negative.
//
// `Lightness` inverts only how bright each pixel is and puts the ORIGINAL hue back
// on top. White pages go black and black text goes white — but a red error badge
// stays red instead of turning cyan. For actually using a desktop inverted, this
// is the one that remains readable.
//
// @prop amount float default=1.0 min=0.0 max=1.0 step=0.01 label="Amount" group="Invert"
// @prop mode   int   default=1 choices="Full,Lightness"    label="Mode"   group="Invert"

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

    var flipped: vec3<f32>;
    if (i32(round(pc.params[0].y)) == 1) {
        // Keep the chroma, flip the luma: rebuild the colour around the inverted
        // brightness. `max(l, eps)` guards pure black, whose chroma is undefined.
        let l = dot(src, vec3<f32>(0.2126, 0.7152, 0.0722));
        let chroma = src - vec3<f32>(l);
        flipped = clamp(vec3<f32>(1.0 - l) + chroma, vec3<f32>(0.0), vec3<f32>(1.0));
    } else {
        flipped = vec3<f32>(1.0) - src;
    }

    var col = mix(src, flipped, amount);
    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

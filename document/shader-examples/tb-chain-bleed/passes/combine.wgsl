// TB — the chain, the last pass: put the pyramid back together.
//
// THE CONTROL THAT MAKES THE CHAIN BELIEVABLE
// -------------------------------------------
// `Show` is why this bundle exists rather than another bloom. Set it to
// "Level 1" and the screen becomes the half-res blur, alone, full-screen; then
// "Level 2", then "Level 3". You are looking directly at each rung of the
// pyramid, and the three are visibly different sizes of blur — which is the only
// honest way to demonstrate that a chain of targets did anything.
//
// A bundle whose effect you cannot see is a bundle you cannot debug, and a graph
// you cannot inspect is one you have to take on trust. One `int` prop with
// `choices` buys both.
//
// It is also the only `int` prop in the tree besides `tb-window-xray`'s, and the
// only `bool` anywhere (`extract.wgsl`'s `edges`) — the `@prop` parser accepts
// `float`, `int`, `bool`, `vec2/3/4` and `color`, but ONE SLOT is reserved per
// prop, so a multi-component value only ever delivers its first lane. Use
// `float`, `int` and `bool`; the others parse and then quietly disappoint.
//
// WEIGHTS PER LEVEL
// -----------------
// Three sliders rather than one strength, so the shape of the glow is
// adjustable, not just its amount: the tight level is a halo around edges, the
// wide one is a broad wash across the whole screen. Zero two of them and you have isolated a
// level without leaving the composite — the same inspection `Show` gives, but
// blended.
//
// @prop show     int   default=0 choices="Composite,Level 1 (½),Level 2 (¼),Level 3 (⅛),Bleed only" label="Show" group="Inspect"
// @prop strength float default=1.10 min=0.0 max=4.0 step=0.01 label="Strength"       group="Bleed"
// @prop tight    float default=1.00 min=0.0 max=2.0 step=0.01 label="Tight (½)"      group="Bleed"
// @prop medium   float default=0.75 min=0.0 max=2.0 step=0.01 label="Medium (¼)"     group="Bleed"
// @prop wide     float default=0.55 min=0.0 max=2.0 step=0.01 label="Wide (⅛)"       group="Bleed"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var l1: texture_2d<f32>;
@group(0) @binding(3) var l2: texture_2d<f32>;
@group(0) @binding(4) var l3: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let show = i32(pc.params[0].x + 0.5);
    let p_strength = pc.params[0].y;
    // Prop #i drives float slot i, in the order this file declares them:
    // show, strength, tight, medium, wide. The names are deliberately NOT `l1`
    // etc. — those are the manifest's input bindings, a separate namespace, and
    // reusing them here reads like a connection that does not exist.
    let w_tight = pc.params[0].z;
    let w_medium = pc.params[0].w;
    let w_wide = pc.params[1].x;

    let a = textureSampleLevel(l1, samp, uv, 0.0).rgb;
    let b = textureSampleLevel(l2, samp, uv, 0.0).rgb;
    let c = textureSampleLevel(l3, samp, uv, 0.0).rgb;

    // Inspection modes: each level alone, full-screen and unweighted, so what you
    // are seeing is the target's own contents and not a tuned view of them.
    var col: vec3<f32>;
    if (show == 1) {
        col = a;
    } else if (show == 2) {
        col = b;
    } else if (show == 3) {
        col = c;
    } else {
        let bleed = (a * w_tight + b * w_medium + c * w_wide) * p_strength;
        if (show == 4) {
            col = bleed;
        } else {
            col = textureSampleLevel(scene, samp, uv, 0.0).rgb + bleed;
        }
    }

    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

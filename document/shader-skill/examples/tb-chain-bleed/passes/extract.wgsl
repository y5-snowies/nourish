// TB — the chain, pass 1: what is allowed to glow.
//
// Reads the composited desktop and keeps only what is brighter than `threshold`,
// at HALF resolution. Downscaling here rather than later is most of why a
// three-level blur is affordable at all: every pass after this one works on a
// quarter of the pixels, and the one after that a sixteenth.
//
// `edges` is the reason this bundle is worth looking at rather than just
// measuring. With it off you get an ordinary brightness bloom, which on a dark
// desktop is exactly the "nothing appears to happen" that makes a chain
// impossible to believe in. With it on, the extract is a LOCAL CONTRAST test —
// bright next to dark — so window borders, text and panel edges light up on any
// wallpaper, and the effect is unmistakable the moment you enable it.
//
// @prop threshold float default=0.55 min=0.0 max=2.0 step=0.01 label="Threshold"    group="Bleed"
// @prop edges     bool  default=true                           label="Edges, not brightness" group="Bleed"
// @prop knee      float default=0.35 min=0.0 max=1.0 step=0.01 label="Soft knee"    group="Bleed"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var src: texture_2d<f32>;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    // `res` is THIS pass's target — half the screen. The engine pushes each
    // intermediate the size of what it draws into, so a scaled pass needs no
    // correction of its own.
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let p_threshold = pc.params[0].x;
    let p_edges = pc.params[0].y > 0.5;
    let p_knee = pc.params[0].z;

    let c = textureSampleLevel(src, samp, uv, 0.0).rgb;
    var weight = luma(c);

    if (p_edges) {
        // Local contrast: how much brighter this pixel is than its neighbourhood.
        // One texel at THIS level is two screen pixels, which is about the width
        // of the chrome we want to catch.
        let o = 1.0 / res;
        var around = 0.0;
        around = around + luma(textureSampleLevel(src, samp, uv + vec2<f32>( o.x, 0.0), 0.0).rgb);
        around = around + luma(textureSampleLevel(src, samp, uv + vec2<f32>(-o.x, 0.0), 0.0).rgb);
        around = around + luma(textureSampleLevel(src, samp, uv + vec2<f32>(0.0,  o.y), 0.0).rgb);
        around = around + luma(textureSampleLevel(src, samp, uv + vec2<f32>(0.0, -o.y), 0.0).rgb);
        weight = abs(weight - around * 0.25) * 4.0;
    }

    // Soft knee, so the effect fades in around the threshold instead of switching
    // on at it — a hard cutoff makes a slider feel broken near its useful range.
    let lo = p_threshold * (1.0 - p_knee);
    let keep = smoothstep(lo, p_threshold + 0.0001, weight);
    return vec4<f32>(c * keep, 1.0);
}

// TB — the chain, the step between levels: halve the resolution.
//
// A 4-tap box, sampled at the offsets that make bilinear give a clean 2:1
// reduction. Used TWICE — half → quarter and quarter → eighth — which is the
// other half of the "one file, many passes" idea: unlike `blur.wgsl` it needs no
// shader-def, because nothing about it differs between the two uses. The output
// target's `scale` is what makes it a downsample.
//
// This is worth its own pass rather than folding into the blur. Blurring at full
// rate and then throwing pixels away costs four times as much as throwing them
// away first, and a pyramid exists to not do that.
//
// CADENCE COVERS A SUB-CHAIN, NOT A LINK
// --------------------------------------
// The third level runs every SECOND frame, and that means all three of its
// passes do — this downsample included. Throttling only the blurs left this one
// refreshing `l3_a` every frame while the blur that owns it refreshed every
// other, so the target alternated between the raw downsample and the blurred
// one. On screen: a per-frame flicker in the widest halo, in proportion to the
// `Wide` weight, which reads as the effect being unstable rather than as the
// graph being wrong.
//
// The engine refuses that shape now (`shader.graph::plan` — every pass writing a
// target must share its cadence), so the mistake is a load error with both pass
// names in it rather than something to notice by eye.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    // A quarter-texel at the DESTINATION is a half-texel at the source, which is
    // where bilinear averages two source texels per tap — four taps, sixteen
    // source texels, no banding.
    let o = 0.25 / res;
    var acc = textureSampleLevel(src, samp, uv + vec2<f32>( o.x,  o.y), 0.0).rgb;
    acc = acc + textureSampleLevel(src, samp, uv + vec2<f32>(-o.x,  o.y), 0.0).rgb;
    acc = acc + textureSampleLevel(src, samp, uv + vec2<f32>( o.x, -o.y), 0.0).rgb;
    acc = acc + textureSampleLevel(src, samp, uv + vec2<f32>(-o.x, -o.y), 0.0).rgb;
    return vec4<f32>(acc * 0.25, 1.0);
}

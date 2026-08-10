// TB — `storage`, pass 2 of 3: the scatter.
//
// Every pixel of the composited desktop adds one to the bin its OWN BRIGHTNESS
// selects. A pixel at the top of the screen therefore writes to a word that a
// pixel at the bottom also writes to — that is a scatter, and it is the thing a
// persistent target cannot express: rendering only ever writes the pixel being
// shaded.
//
// `atomicAdd`, not `+= 1`: every invocation in the frame targets the same 64
// words. A plain read-modify-write would race and the totals would be an
// arbitrary subset of the pixels — a histogram that is merely wrong rather than
// obviously broken.
//
// Runs at quarter scale (its `scratch` output), so it samples one pixel in
// sixteen. A histogram is a distribution, and a sixteenth of two million pixels
// is a very good estimate of it for a sixteenth of the cost.

const BINS: u32 = 64u;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

struct Bins { count: array<atomic<u32>, 256>, };
@group(2) @binding(0) var<storage, read_write> bins: Bins;

/// Rec.709 luma — the same weighting `mp-grayscale` uses, so "brightness" means
/// one thing across the shipped set.
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    // `res` is THIS PASS'S target, not the screen: the engine pushes each
    // intermediate pass the size of what it is drawing into, so a quarter-scale
    // pass sees quarter-scale numbers and its UV needs no correction.
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let col = textureSampleLevel(scene, samp, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
    // The index comes from the VALUE here, not from the position. That is the
    // whole difference from a render target.
    let bin = min(u32(clamp(luma(col), 0.0, 0.999) * f32(BINS)), BINS - 1u);
    atomicAdd(&bins.count[bin], 1u);
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

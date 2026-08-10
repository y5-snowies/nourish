// dragons, pass 3 — isolate what is hot enough to bloom.
//
// Runs on a quarter-scale target, so it costs a sixteenth of the pixels of the
// picture it is reading. That downscale is the whole reason a wide glow around
// the fire is affordable at all: the blur that follows works on this, not on
// the full-resolution frame.
//
// Soft knee rather than a hard threshold — a hard cut makes the bloom pop on and
// off as the fire flickers across the boundary.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 1>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

const THRESHOLD: f32 = 0.62;
const KNEE: f32 = 0.45;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;

    // Four taps at quarter scale: a box prefilter, so the downsample does not
    // alias the flame into a crawling speckle.
    let o = 0.75 / res;
    var c = textureSampleLevel(scene, samp, uv + vec2<f32>(-o.x, -o.y), 0.0).rgb;
    c = c + textureSampleLevel(scene, samp, uv + vec2<f32>(o.x, -o.y), 0.0).rgb;
    c = c + textureSampleLevel(scene, samp, uv + vec2<f32>(-o.x, o.y), 0.0).rgb;
    c = c + textureSampleLevel(scene, samp, uv + vec2<f32>(o.x, o.y), 0.0).rgb;
    c = c * 0.25;

    let lum = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let w = smoothstep(THRESHOLD - KNEE, THRESHOLD + KNEE, lum);
    return vec4<f32>(c * w, 1.0);
}

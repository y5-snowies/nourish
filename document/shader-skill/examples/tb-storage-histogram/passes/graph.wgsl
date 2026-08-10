// TB — `storage`, pass 3 of 3: read the tally back and draw it.
//
// The desktop with a live luminance histogram along the bottom. Open a dark
// window and the left bars grow; a white one and the right bars grow. Drag a
// window around and the shape shifts continuously.
//
// This reads what `tally` wrote THIS frame — the engine's storage barrier between
// intermediate passes is what makes that safe, and it is the reason the three
// stages are three passes rather than one.
//
// @prop height  float default=0.22 min=0.02 max=0.60 step=0.01 label="Graph height" group="Graph"
// @prop gain    float default=1.0  min=0.1  max=8.0  step=0.1  label="Bar gain"     group="Graph"
// @prop dim     float default=0.35 min=0.0  max=1.0  step=0.01 label="Dim desktop"  group="Graph"

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

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let p_height = pc.params[0].x;
    let p_gain = pc.params[0].y;
    let p_dim = pc.params[0].z;

    var col = textureSampleLevel(scene, samp, uv, 0.0).rgb;

    let band = clamp(p_height, 0.02, 0.9);
    if (uv.y > 1.0 - band) {
        let slot = min(u32(uv.x * f32(BINS)), BINS - 1u);
        let n = f32(atomicLoad(&bins.count[slot]));
        // Normalised against the number of samples the tally actually took (a
        // quarter of the frame on each axis), so the shape is the same at every
        // resolution. `sqrt` because a desktop is overwhelmingly mid-grey and a
        // linear scale shows one spike and 63 empty slots.
        let samples = max(res.x * 0.25 * res.y * 0.25, 1.0);
        let share = sqrt(n / samples) * p_gain;
        let bar = (uv.y - (1.0 - band)) / band;
        col = col * (1.0 - p_dim);
        if (1.0 - bar < clamp(share, 0.0, 1.0)) {
            // Coloured by WHICH bin, so the graph reads left-to-right as
            // dark-to-light and a mis-indexed bin shows as a colour out of order.
            let hue = f32(slot) / f32(BINS);
            col = col + vec3<f32>(0.25 + hue * 0.75, 0.45, 1.0 - hue * 0.6) * 0.8;
        }
        if (bar > 0.985) {
            col = col + vec3<f32>(0.15);
        }
    }

    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

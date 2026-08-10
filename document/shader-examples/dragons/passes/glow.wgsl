// dragons, pass 6 — the frame you actually see: scene + firelight, tone-mapped.
//
// This is the pass that writes `output`, so it is the one that does the sRGB
// encode. The band pass deliberately does not.
//
// Nothing here DISPLACES the picture — the glow is added, never warped. That is
// why the bundle needs no `hit` block: every pixel is still where the compositor
// thinks it is, so clicks land on what you see.
//
// @prop bloomamt float default=0.85 min=0.00 max=2.00 step=0.05 label="Firelight bloom" group="Fire"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 1>,
};
var<immediate> pc: Push;

// Sorted by binding NAME: bloom, then scene.
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var scene: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;

    let amount = pc.params[0].x;
    var col = textureSampleLevel(scene, samp, uv, 0.0).rgb;
    let glow = textureSampleLevel(bloom, samp, uv, 0.0).rgb;

    // Firelight is warm, so bias the spread warm as it spreads — cool bloom off
    // an orange source is the tell that a glow was bolted on.
    let warm = vec3<f32>(1.00, 0.72, 0.46);
    col = col + glow * warm * amount;

    // Soft shoulder. The fire runs well above 1.0 on purpose; rolling it off
    // keeps a hot core reading as bright rather than as a flat white blob.
    col = col / (1.0 + col * 0.42);
    col = col * 1.18;

    col = max(col, vec3<f32>(0.0));
    if (pc.lock_alpha.z > 0.5) {
        col = pow(col, vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

// storm-forest, passes 2 and 3 — the separable blur that puts the forest out of
// focus. One file, compiled twice: the manifest passes `HORIZONTAL` to one of
// them, so the axis is the only difference and there is no second copy to keep
// in step.
//
// It runs on a half-scale target, so a radius here is worth twice as much on
// screen as the number suggests. Keep it modest: blur wider than the canopy
// detail and the tree line stops being a tree line.
//
// @prop defocus float default=1.20 min=0.0 max=6.0 step=0.05 label="Forest blur" group="Forest"

#ifdef HORIZONTAL
const AXIS: vec2<f32> = vec2<f32>(1.0, 0.0);
#else
const AXIS: vec2<f32> = vec2<f32>(0.0, 1.0);
#endif

// Wide kernel: this is meant to read as depth of field, not as softening.
const TAPS: i32 = 4;   // @optimized 2

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 1>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var src: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let radius = max(pc.params[0].x, 0.0);

    if (radius <= 0.0) {
        return vec4<f32>(textureSampleLevel(src, samp, uv, 0.0).rgb, 1.0);
    }

    let step = AXIS * radius / res;
    let k = array<f32, 6>(0.196, 0.155, 0.104, 0.059, 0.028, 0.011);

    var acc = textureSampleLevel(src, samp, uv, 0.0).rgb * 0.214;
    var w = 0.214;
    for (var i = 0; i < TAPS; i = i + 1) {
        let d = step * f32(i + 1);
        let kw = k[i];
        acc = acc + textureSampleLevel(src, samp, uv + d, 0.0).rgb * kw;
        acc = acc + textureSampleLevel(src, samp, uv - d, 0.0).rgb * kw;
        w = w + kw * 2.0;
    }
    return vec4<f32>(acc / w, 1.0);
}

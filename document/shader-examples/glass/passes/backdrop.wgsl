// glass pass 1/2 — "backdrop" (before-content). A calm, pan-linked gradient so
// there is a visible wallpaper behind the windows for the glass to frost. Tracks
// the camera (world = uv - pan*0.0006) so panning slides it.
//
// @prop hue float default=0.58 min=0.0 max=1.0 step=0.01 label="Backdrop hue" group="Glass"

struct Push {
    res_zoom_time: vec4<f32>,     // xy = resolution, z = zoom, w = time
    pan_flow: vec4<f32>,          // xy = pan, zw = flow_offset
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,  // @prop slot 0.x = hue
};
var<immediate> pc: Push;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = pc.res_zoom_time.xy;
    let pan = pc.pan_flow.xy;
    let hue = pc.params[0].x; // @prop hue

    var uv = frag.xy / max(res, vec2<f32>(1.0));
    let world = uv - pan * 0.0006;

    let top = mix(vec3<f32>(0.06, 0.09, 0.16), vec3<f32>(0.16, 0.11, 0.24), hue);
    let bot = vec3<f32>(0.02, 0.03, 0.07);
    var col = mix(top, bot, uv.y);

    // Soft moving diagonal bands so the frost has something to smear.
    let band = 0.5 + 0.5 * sin((world.x + world.y) * 24.0);
    col = col + vec3<f32>(0.04, 0.05, 0.07) * band;
    return vec4<f32>(col, 1.0);
}

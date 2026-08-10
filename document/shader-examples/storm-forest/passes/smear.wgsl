// storm-forest, pass 5/5 — motion blur, grade, and the swapchain write.
//
// The only pass that touches `output` for real, so the sRGB encode lives here
// and nowhere else.
//
// The smear is driven by two things at once:
//   * camera pan  — `lock_alpha.w` carries the packed pan velocity;
//   * the shake   — sampled straight from lib/quake.wgsl by differencing the
//                   displacement across one frame, so the blur direction is the
//                   direction the windows are actually being thrown, not a
//                   guess. Standing still during a strike still smears.
//
// It is directional rather than a disk: a rattle is a line, and blurring it
// radially just makes the desktop look soft instead of shaken.
//
// @prop motion   float default=1.00  min=0.0  max=2.5  step=0.01  label="Motion blur"           group="Motion"
// @prop shake    float default=0.011 min=0.0  max=0.05 step=0.001 label="Lightning shake"       group="Lightning"
// @prop period   float default=9.00  min=2.0  max=40.0 step=0.5   label="Seconds between bolts" group="Lightning"
// @prop vignette float default=0.45  min=0.0  max=1.5  step=0.01  label="Vignette"              group="Motion"

#import storm::quake::quake_offset

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 1>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var cur: texture_2d<f32>;    // "cur"  -> content
@group(0) @binding(2) var prev: texture_2d<f32>;   // "prev" -> history

const TAPS: i32 = 6;   // @optimized 3

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let t = pc.res_zoom_time.w;

    let motion = pc.params[0].x;
    let shake = pc.params[0].y;
    let period = pc.params[0].z;
    let vig = pc.params[0].w;

    var col = textureSampleLevel(cur, samp, uv, 0.0).rgb;

    if (motion > 0.0) {
        // Pan velocity, two snorm16 halves in one lane.
        let vel = unpack2x16snorm(bitcast<u32>(pc.lock_alpha.w)) * 16384.0;
        let pan_s = length(vel) * motion * 0.020;
        let pan_blend = 0.85 * pan_s / (1.0 + pan_s);

        // Shake velocity: the same displacement the band pass drew with,
        // differenced over one frame. Index 0 picks up the global jolt, which
        // is the part every window shares.
        let sv = (quake_offset(shake, period, t, 0.0)
                - quake_offset(shake, period, t - 0.0167, 0.0)) * res;
        let shake_s = length(sv) * motion * 0.075;
        let shake_blend = 0.80 * shake_s / (1.0 + shake_s);

        let blend = max(pan_blend, shake_blend);
        if (blend > 0.004) {
            var dir = vel;
            if (shake_blend >= pan_blend) {
                dir = sv;
            }
            let dl = length(dir);
            if (dl > 0.0001) {
                dir = dir / dl;
            } else {
                dir = vec2<f32>(1.0, 0.0);
            }

            let radius = (4.0 + 42.0 * blend) * motion;
            var acc = textureSampleLevel(prev, samp, uv, 0.0).rgb;
            var w = 1.0;
            for (var i = 1; i <= TAPS; i = i + 1) {
                let d = dir * (radius * f32(i) / f32(TAPS)) / res;
                acc = acc + textureSampleLevel(prev, samp, uv + d, 0.0).rgb;
                acc = acc + textureSampleLevel(prev, samp, uv - d, 0.0).rgb;
                w = w + 2.0;
            }
            col = mix(col, acc / w, blend);
        }
    }

    // A cold, closed-in grade: the storm should feel like weather pressing on
    // the edges of the screen.
    if (vig > 0.0) {
        let c = (uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(res.x / max(res.y, 1.0), 1.0);
        let d = dot(c, c);
        col = col * (1.0 - clamp(d * 0.55 * vig, 0.0, 0.85));
    }

    col = max(col, vec3<f32>(0.0));
    if (pc.lock_alpha.z > 0.5) {
        col = pow(col, vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

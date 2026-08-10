// TB STRESS 20 — "tb-cadence", pass 2/2. Runs EVERY frame (the `output` pass has
// no cadence — `plan()` rejects one, since skipping it would leave the frame with
// no picture rather than a stale one).
//
// Draws its own arm at full rate in cyan and composites the held orange arm from
// the `slow` target on top. Watch the two:
//
//   cyan sweeps smoothly, orange jumps in steps  -> cadence is working
//   both smooth                                  -> cadence ignored
//   orange absent                                -> the held target is being
//                                                   cleared between runs, which
//                                                   would defeat the whole point
//
// Under triple buffering the tick is PER PANE, so on a multi-monitor setup each
// monitor's orange arm should step at its own rate rather than in lockstep — a
// shared counter would have made a `cadence: 8` pass fire at a rate that depended
// on the other monitors.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var held: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let p = (uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(res.x / max(res.y, 1.0), 1.0);

    var col = vec3<f32>(0.04, 0.05, 0.09) + 0.05 * uv.y;
    // Dial face.
    let r = length(p);
    col = col + vec3<f32>(0.10, 0.12, 0.18) * smoothstep(0.008, 0.0, abs(r - 0.42));

    // Full-rate arm (cyan).
    let ang = atan2(p.y, p.x);
    let d = abs(fract((ang - t * 1.2) / 6.2831853 + 0.5) - 0.5);
    let arm = smoothstep(0.02, 0.0, d) * smoothstep(0.42, 0.10, r);
    col = col + vec3<f32>(0.15, 0.9, 1.0) * arm;

    // The held arm, produced every 8th frame.
    col = col + textureSample(held, samp, uv).rgb;
    return vec4<f32>(col, 1.0);
}

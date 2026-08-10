// TB STRESS 5/10 — "tb-after-history", pass 2/2 (AFTER-CONTENT, reads `history`).
//
// DIAGNOSTIC for the history contract. It draws |content - history| amplified in
// red on top of the scene, so the temporal gap is directly visible: the wider
// the red ghost trailing the moving bar, the further `history` is behind.
//
// Why this matters for triple buffering: `history` is defined today as "the
// PREVIOUS FRAME's composited content". Once content is paced through a ring to
// a worker, it becomes "content from N ticks ago", with N varying by rate,
// cadence, ceiling AND hardware. This bundle makes N visible. If the red ghost
// widens or pulses when you enable TB or change the rate, the history contract
// has drifted and any temporal effect (motion-blur) is now machine-dependent.
//
// `history` also forces the offscreen path on (keep_history), independently of
// the after-content band — so this bundle exercises both gates at once.
//
// @prop gain float default=6.0 min=1.0 max=24.0 step=0.5 label="Delta gain" group="TB stress"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var cur: texture_2d<f32>;
@group(0) @binding(2) var prev: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let gain = pc.params[0].x;
    let a = textureSample(cur, samp, uv).rgb;
    let b = textureSample(prev, samp, uv).rgb;
    let d = clamp(length(a - b) * gain, 0.0, 1.0);
    return vec4<f32>(mix(a, vec3<f32>(1.0, 0.15, 0.1), d * 0.8), 1.0);
}

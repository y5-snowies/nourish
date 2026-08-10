// STAGE 4 VALIDATOR, pass 2/2 — the TAIL band, on the WORKER.
//
// `place: "worker"` is the opt-in. Without it the after band stays inline: the
// manifest cannot tell which band is expensive, and defaulting to the tail would
// offload a trivial decorate pass while leaving a heavy background behind.
//
// What makes this stage different from everything else here: the worker is not
// producing a background. It samples the band the COMPOSITOR composited —
// background AND windows, drawn by the engine's own path — decorates it, and the
// compositor presents that instead of running this pass itself. One sample,
// whatever the window count, with the engine's AA, HDR, damage and occlusion all
// still applied. Stage 3 could express this too, but only by recompositing every
// window in the shader first.
//
// The decoration is deliberately unmistakable and POSITION-DEPENDENT: a strong
// diagonal wash plus a bright moving band, applied to the whole picture including
// windows and UI.
//
// | What you see | Meaning |
// |---|---|
// | wash + moving band over EVERYTHING, windows included | stage 4 works |
// | wash present but windows undecorated | the band is not the world band |
// | no wash | the after pass ran nowhere, or the band was never published |
// | frozen picture | the compositor is presenting a stale band — the withdraw path failed |
//
// Expect the decoration to lag input by ~2 frames: `content` out, decorated band
// back. That staleness IS the stage's cost, and it applies to the whole picture,
// which is why the effect stays coherent rather than desynchronised.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    var col = textureSample(scene, samp, uv).rgb;

    // Diagonal wash — everything the compositor composited, tinted.
    let d = fract((uv.x + uv.y) * 0.5 - t * 0.05);
    col = col * mix(vec3<f32>(1.15, 0.85, 0.75), vec3<f32>(0.75, 0.95, 1.25), d);

    // A bright band sweeping across the whole picture, windows included.
    let sweep = fract(t * 0.15);
    let band = smoothstep(0.06, 0.0, abs(fract(uv.x - sweep) - 0.5) - 0.44);
    col = col + vec3<f32>(0.35, 0.30, 0.15) * band;
    return vec4<f32>(col, 1.0);
}

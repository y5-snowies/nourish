// WORKER-DERIVED `history`. The row this bundle exists to prove.
//
// `history` is NOT transported. The compositor produces its own history by
// copying `content` at frame end; the worker receives `content` already (stage
// 4), so it takes the same copy on its own device — see `worker.history`. No
// second export, no extra fd, no dependency on when the compositor rotates its
// copy.
//
// THE SEMANTIC DIFFERENCE IS THE POINT OF LOOKING AT THIS
// ------------------------------------------------------
// Inline, `history` is the previous FRAME. Here it is the previous PASS, and the
// worker runs at its own rate — so the SAME decay constant produces a LONGER,
// coarser trail than the `-inline` twin. Run both back to back: if they look
// identical, the worker is running at compositor rate and the comparison proves
// nothing; if the offloaded one smears further, that is the worker's clock, and
// it is the cost of offloading a temporal effect rather than a bug to file.
//
// | What you see | Meaning |
// |---|---|
// | a comet trail behind the moving band | worker history works |
// | no trail, just the band | `history` bound null — derivation did not run |
// | full-screen garbage on the first frames | the prime() clear did not happen |
// | trail longer than the inline twin's | correct, and expected — see above |
//
// @prop decay float default=0.88 min=0.5 max=0.98 step=0.01 label="Trail decay" group="Worker history"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var prev: texture_2d<f32>;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let decay = clamp(pc.params[0].x, 0.0, 0.98);
    let now = textureSample(scene, samp, uv).rgb;
    let old = textureSample(prev, samp, uv).rgb;
    // max(), not mix(): a trail that only ever brightens is unmistakable, where a
    // blend of two similar frames is easy to mistake for no effect at all.
    return vec4<f32>(max(now, old * decay), 1.0);
}

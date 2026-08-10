// TB STRESS 21 — "tb-chain-proof", the verdict pass.
//
// Samples the end of the chain and checks each stage's signature. Draws THREE
// large lights across the middle of the screen:
//
//   all three GREEN  -> every pass ran, in order, reading the right target.
//                       The multipass graph is genuinely working.
//   light k RED      -> stage k's signature is missing or wrong. Either that pass
//                       did not run, or the pass after it read the wrong target
//                       and overwrote the chain.
//   all three RED    -> the chain never happened; you are almost certainly seeing
//                       a single-pass fallback (check the log for a load error).
//
// A wide banner underneath repeats the verdict in one colour so it is readable
// across a room, and the raw strips are drawn along the top for eyeballing.
//
// This is deliberately ugly and binary. A pretty bundle cannot tell you whether
// its passes ran; this one can, which is what makes it the reference for an
// offload A/B — compare `tb-chain-proof` against `tb-chain-proof-inline` and a
// difference means the WORKER path broke the chain, not that a glow looks off.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var chain: texture_2d<f32>;

fn strip_centre(k: f32) -> f32 {
    return 0.10 + k * 0.08 + 0.025;
}

// A signature matches when its own channel is high and the other two are low.
fn ok(c: vec3<f32>, k: u32) -> bool {
    let want = select(select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), k == 1u),
                      vec3<f32>(1.0, 0.0, 0.0), k == 0u);
    return distance(c, want) < 0.25;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;

    // Decode the three signatures once.
    let c0 = textureSample(chain, samp, vec2<f32>(0.5, strip_centre(0.0))).rgb;
    let c1 = textureSample(chain, samp, vec2<f32>(0.5, strip_centre(1.0))).rgb;
    let c2 = textureSample(chain, samp, vec2<f32>(0.5, strip_centre(2.0))).rgb;
    let g0 = ok(c0, 0u);
    let g1 = ok(c1, 1u);
    let g2 = ok(c2, 2u);
    let all = g0 && g1 && g2;

    var col = vec3<f32>(0.05, 0.05, 0.07);

    // Raw strips along the top, for eyeballing what the chain actually carried.
    if (uv.y < 0.34) {
        col = textureSample(chain, samp, uv).rgb * 0.85;
    }

    // Three big lights.
    if (uv.y > 0.42 && uv.y < 0.62) {
        let cell = uv.x * 3.0;
        let i = u32(floor(cell));
        let d = abs(fract(cell) - 0.5);
        if (d < 0.30) {
            let good = select(select(g2, g1, i == 1u), g0, i == 0u);
            col = select(vec3<f32>(0.85, 0.05, 0.05), vec3<f32>(0.05, 0.9, 0.2), good);
        }
    }

    // Verdict banner.
    if (uv.y > 0.68 && uv.y < 0.80) {
        col = select(vec3<f32>(0.55, 0.0, 0.0), vec3<f32>(0.0, 0.5, 0.12), all);
    }
    return vec4<f32>(col, 1.0);
}

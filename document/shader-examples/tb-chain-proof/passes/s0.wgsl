// TB STRESS 21 — "tb-chain-proof", stage 0 of 3.
//
// Writes its SIGNATURE (pure red) into strip 0 and nothing else. Each later stage
// copies what it read and adds its own strip, so the final target carries one
// strip per stage that actually ran, in the right order, having read the right
// input. `report` decodes them into pass/fail lights.
//
// This exists because a pretty multipass bundle cannot verify itself: bloom looks
// broadly the same whether its five passes ran or a single fallback did, so it
// proves nothing about the chain. Here a skipped pass, a mis-wired target index
// or a chain that silently collapsed shows up as a RED LIGHT, not as a slightly
// different glow.
struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

// Strip k spans y in [0.10 + k*0.08, +0.05]; sampled at its centre, so a
// half-resolution intermediate still round-trips it intact.
fn in_strip(uv: vec2<f32>, k: f32) -> bool {
    let lo = 0.10 + k * 0.08;
    return uv.y >= lo && uv.y <= lo + 0.05;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    if (in_strip(uv, 0.0)) {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);   // signature 0
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

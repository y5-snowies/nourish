// window-glow pass 2/2 — "glow" (after-content, needs window-rects). Samples the
// composited scene (`content`) and adds a soft bright halo just OUTSIDE each
// window's rectangle, read from the window-rects UBO at @group(1). Because it
// runs after content but below the UI (§8b), the glow sits around windows but
// under the cursor/panels. This is the "glow around a window" effect.
//
// The window rects arrive in UV space (xy = top-left, zw = size), physical / res.
// They cover CLIENT WINDOWS only — iced-world panels/placeholders are excluded,
// so the glow never haloes a placeholder.
//
// @prop intensity float default=1.0  min=0.0 max=4.0 step=0.01 label="Glow intensity" group="Window glow"
// @prop radius    float default=0.05 min=0.0 max=0.3 step=0.005 label="Glow radius"   group="Window glow"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,  // @prop slots: 0 = intensity, 1 = radius
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

struct Windows {
    count: u32,
    _pad: vec3<u32>,
    rects: array<vec4<f32>, 256>,  // xy = origin (uv), zw = size (uv)
};
@group(1) @binding(0) var<uniform> windows: Windows;

// Signed distance to an axis-aligned box (negative inside, 0 on the edge).
fn sd_box(p: vec2<f32>, half: vec2<f32>) -> f32 {
    let d = abs(p) - half;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / pc.res_zoom_time.xy;
    let intensity = pc.params[0].x; // @prop intensity
    let radius = pc.params[0].y;    // @prop radius

    var col = textureSample(scene, samp, uv).rgb;

    // Accumulate the nearest-edge halo from every on-screen window.
    var glow = 0.0;
    let n = min(windows.count, 256u);
    for (var i = 0u; i < n; i = i + 1u) {
        let r = windows.rects[i];
        let center = r.xy + r.zw * 0.5;
        let d = sd_box(uv - center, r.zw * 0.5); // >0 outside the window
        // Ring just outside the edge, fading over `radius`; nothing inside.
        glow = glow + smoothstep(radius, 0.0, d) * step(0.0, d);
    }

    let tint = vec3<f32>(0.35, 0.75, 1.0);
    col = col + tint * min(glow, 1.5) * intensity;
    return vec4<f32>(col, 1.0);
}

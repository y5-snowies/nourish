// TB STRESS 19 — "tb-crt". Old-television tube emulation, after-content.
//
// Same family as `vignette` — an after-content pass over the composited scene —
// but where vignette only ATTENUATES what is already there, this RESAMPLES the
// whole picture through a curved screen. That difference is the point:
//
//   * every output pixel reads `content` at a DIFFERENT place (barrel warp), so
//     nothing is a 1:1 blit any more and the pass cannot be damage-scissored
//     meaningfully — it is the honest worst case for the after-content path;
//   * three taps per pixel for chromatic aberration, so it is bandwidth-bound on
//     a full-res target rather than ALU-bound;
//   * the tube mask is computed at output-pixel frequency, which is exactly where
//     fractional scaling and non-native resolutions produce moire — a good canary
//     for the scaled-target sampling that `tb-mixed-scale` tests structurally.
//
// Effects, in tube order: barrel curvature -> bezel cutoff -> RGB channel
// separation -> aperture-grille mask -> scanlines -> rolling refresh bar ->
// vignette -> mild bloom -> flicker.
//
// @prop curve    float default=0.16 min=0.0 max=0.60 step=0.01  label="Tube curvature"  group="CRT"
// @prop scan     float default=0.35 min=0.0 max=1.00 step=0.01  label="Scanline depth"  group="CRT"
// @prop mask     float default=0.30 min=0.0 max=1.00 step=0.01  label="Aperture mask"   group="CRT"
// @prop aberr    float default=1.60 min=0.0 max=6.00 step=0.10  label="Colour bleed px" group="CRT"
// @prop roll     float default=0.06 min=0.0 max=0.50 step=0.01  label="Refresh roll"    group="CRT"
// @prop flicker  float default=0.03 min=0.0 max=0.30 step=0.01  label="Flicker"         group="CRT"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

// Barrel distortion about the centre. Returns warped UV; callers test for the
// bezel by checking the result against [0,1].
fn warp(uv: vec2<f32>, k: f32) -> vec2<f32> {
    let c = uv * 2.0 - vec2<f32>(1.0);
    let r2 = dot(c, c);
    return (c * (1.0 + k * r2)) * 0.5 + vec2<f32>(0.5);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;

    let curve   = pc.params[0].x;
    let scan    = pc.params[0].y;
    let mask_k  = pc.params[0].z;
    let aberr   = pc.params[0].w;
    let roll    = pc.params[1].x;
    let flick   = pc.params[1].y;

    let w = warp(uv, curve);

    // Bezel: outside the tube is unlit glass, with a soft edge so the cutoff is
    // not a hard alias.
    if (w.x < -0.02 || w.x > 1.02 || w.y < -0.02 || w.y > 1.02) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let inx = smoothstep(0.0, 0.012, w.x) * smoothstep(0.0, 0.012, 1.0 - w.x);
    let iny = smoothstep(0.0, 0.012, w.y) * smoothstep(0.0, 0.012, 1.0 - w.y);
    let bezel = inx * iny;

    // Chromatic aberration: the three guns land slightly apart, more so toward
    // the edges, like a mis-converged tube.
    let off = (w - vec2<f32>(0.5)) * (aberr / max(res.x, 1.0));
    let r = textureSampleLevel(scene, samp, w + off, 0.0).r;
    let g = textureSampleLevel(scene, samp, w, 0.0).g;
    let b = textureSampleLevel(scene, samp, w - off, 0.0).b;
    var col = vec3<f32>(r, g, b);

    // Aperture grille: RGB stripes at output-pixel frequency.
    let stripe = u32(frag.x) % 3u;
    var m = vec3<f32>(1.0 - mask_k);
    if (stripe == 0u) { m.r = 1.0 + mask_k * 0.6; }
    if (stripe == 1u) { m.g = 1.0 + mask_k * 0.6; }
    if (stripe == 2u) { m.b = 1.0 + mask_k * 0.6; }
    col = col * m;

    // Scanlines — two output rows per simulated line.
    let line = 0.5 + 0.5 * cos(frag.y * 3.14159265);
    col = col * (1.0 - scan * line);

    // Rolling refresh bar drifting up the tube.
    let bar = fract(w.y - t * 0.12);
    col = col * (1.0 + roll * smoothstep(0.06, 0.0, bar));

    // Cheap bloom: the picture's own brightness haloing into the shadow mask.
    let lum = dot(col, vec3<f32>(0.299, 0.587, 0.114));
    col = col + col * smoothstep(0.55, 1.0, lum) * 0.35;

    // Tube vignette + mains flicker.
    let d = length((w - vec2<f32>(0.5)) * vec2<f32>(res.x / max(res.y, 1.0), 1.0));
    col = col * mix(1.0, 0.35, smoothstep(0.45, 1.05, d));
    col = col * (1.0 - flick * (0.5 + 0.5 * sin(t * 96.0)));

    return vec4<f32>(col * bezel, 1.0);
}

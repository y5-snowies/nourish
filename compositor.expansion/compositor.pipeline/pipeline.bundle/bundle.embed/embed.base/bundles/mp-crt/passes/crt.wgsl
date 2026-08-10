// "mp-crt" — THE TUBE, OVER THE WHOLE DESKTOP.
//
// Same effects as the `crt-input-map` example — barrel warp, scanlines, corner
// falloff, and a pointer corrected through the very same `hit_inverse` — with
// three differences that make it something to actually run rather than something
// to demonstrate a mechanism with:
//
//  1. THE PARALLAX IS THE PICTURE BEHIND IT. The example draws a flat test
//     backdrop; this shares `mp-parallax`'s scene, so switching to it from any
//     other bundle in the category keeps the same background and only adds glass.
//
//  2. IT SURVIVES AN ULTRAWIDE. The curve is aspect-normalised in `lib/warp.wgsl`
//     — see the note there — so `curve` means the same amount of bend at 16:9 and
//     at 32:9. The example's does not, and folds a 49" panel in on itself.
//
//  3. IT IS AN AFTER-CONTENT PASS, so it curves the COMPOSITED desktop: windows,
//     panels and all, which is what a real tube does — the glass is in front of
//     everything, not behind the windows.
//
//     That is a deliberate trade against `crt-input-map`, which takes the world
//     band with `windows: "world"` and composites it itself. Taking the band buys
//     per-window control (each window could curve on its own axis) and costs the
//     bindless texture array, which needs the device's descriptor-indexing feature
//     — a bundle that refuses to load is worse than one that cannot bend a single
//     window independently. Nothing here wants per-window geometry, so it asks for
//     `composited_scene` and nothing else, and runs anywhere.
//
// HOW TO TELL IT WORKS: curvature is strongest at the corners, so test there.
// Hover a window edge near a corner — the highlight should follow the BENT edge.
// Set `curve` to 0 and the pointer should be pixel-exact everywhere.
//
// @prop curve     float default=0.14 min=0.0 max=0.5  step=0.005 label="Curvature"    group="Tube"
// @prop scan      float default=0.30 min=0.0 max=1.0  step=0.01  label="Scanlines"    group="Tube"
// @prop mask      float default=0.20 min=0.0 max=1.0  step=0.01  label="Aperture"     group="Tube"
// @prop fringe    float default=0.25 min=0.0 max=1.0  step=0.01  label="Fringing"     group="Tube"
// @prop corner    float default=0.45 min=0.0 max=1.5  step=0.01  label="Corner falloff" group="Tube"
// @prop glow      float default=0.15 min=0.0 max=1.0  step=0.01  label="Phosphor glow" group="Tube"
// @prop lines     float default=0.0  min=0.0 max=1080.0 step=8.0 label="Scan resolution" group="Signal"
// @prop levels    float default=0.0  min=0.0 max=64.0 step=1.0   label="Colour levels"  group="Signal"

#import mp::crt::hit_inverse

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
    let uv = frag.xy / res;
    let curve = pc.params[0].x;
    let scan = pc.params[0].y;
    let mask = pc.params[0].z;
    let fringe = pc.params[0].w;
    let corner = pc.params[1].x;
    let glow = pc.params[1].y;
    let lines = pc.params[1].z;
    let levels = pc.params[1].w;

    // THE one displacement. Same function the engine interprets for the pointer.
    var w = hit_inverse(uv, res, vec4<f32>(curve, 0.0, 0.0, 0.0));

    // SCAN RESOLUTION — a set has a fixed line count and a finite spot size, so it
    // cannot show more than that however sharp the signal is. Snap the SAMPLING
    // point to a coarse raster and every tap below inherits it: fringing, glow and
    // the grille all land on the same fat pixels rather than on a sharp image with
    // a filter over it.
    //
    // Quantised AFTER the warp, in tube space, so the raster bends with the glass
    // — a straight pixel grid over a curved picture is the same giveaway that a
    // straight scanline is. Columns follow the panel aspect so pixels stay square,
    // and 0 means native rather than a zero-line raster.
    if (lines > 0.0) {
        let rows = max(lines, 16.0);
        let grid = vec2<f32>(round(rows * res.x / max(res.y, 1.0)), rows);
        w = (floor(w * grid) + vec2<f32>(0.5)) / grid;
    }

    // No surround check, on purpose: `hit_inverse` OVERSCANS (see `lib/warp.wgsl`),
    // so the screen corner lands exactly on the source corner and nothing samples
    // outside the picture at any curvature. The tube fills the display edge to
    // edge — a black border here would be the effect failing to cover the screen,
    // not a feature of it.
    //
    // The example `crt-input-map` does cut off, because it does not overscan; if
    // you ever remove the overscan, put the cutoff back rather than clamping —
    // a clamped sample smears the edge row outward and makes the surround LOOK
    // like content while the pointer map says it came from off-screen.

    // CHROMATIC FRINGING — the three channels land at slightly different radii,
    // strongest where the glass is thickest. Scaled by the aspect-normalised
    // radius so it stays subtle in the middle on any panel.
    var col: vec3<f32>;
    if (fringe > 0.0) {
        let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
        let c = (w - vec2<f32>(0.5)) * a;
        let r = length(c) / max(0.5 * length(a), 0.0001);
        let off = c * (fringe * 0.004 * r * r) / a;
        col = vec3<f32>(
            textureSampleLevel(scene, samp, clamp(w + off, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).r,
            textureSampleLevel(scene, samp, w, 0.0).g,
            textureSampleLevel(scene, samp, clamp(w - off, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).b,
        );
    } else {
        col = textureSampleLevel(scene, samp, w, 0.0).rgb;
    }

    // COLOUR LEVELS — the signal, before anything analog touches it. A set drove
    // its guns from a limited palette, so the banding belongs to the PICTURE and
    // the scanlines, grille and glow then land on top of an already-banded image.
    // Quantising at the end instead would band those effects too, which reads as
    // a posterise filter over a CRT rather than as a CRT with few colours.
    //
    // `levels - 1` steps between 0 and 1, so 2 gives pure on/off per channel and
    // 64 is very nearly continuous. An ordered 2x2 dither breaks the flattest
    // gradients — the parallax backdrop is exactly the smooth field that would
    // otherwise show wide, obvious bands.
    if (levels > 0.0) {
        let n = max(levels, 2.0) - 1.0;
        let cell = vec2<u32>(u32(frag.x) % 2u, u32(frag.y) % 2u);
        var d = 0.0;
        if (cell.x == 0u && cell.y == 0u) { d = -0.375; }
        else if (cell.x == 1u && cell.y == 0u) { d = 0.125; }
        else if (cell.x == 0u && cell.y == 1u) { d = 0.375; }
        else { d = -0.125; }
        col = clamp(round(col * n + d) / n, vec3<f32>(0.0), vec3<f32>(1.0));
    }

    // PHOSPHOR GLOW — bright areas bleed. Four taps, only where it shows.
    if (glow > 0.0) {
        let e = 1.5 / res.y;
        var bloom = vec3<f32>(0.0);
        bloom = bloom + textureSampleLevel(scene, samp, clamp(w + vec2<f32>(e, 0.0), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
        bloom = bloom + textureSampleLevel(scene, samp, clamp(w - vec2<f32>(e, 0.0), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
        bloom = bloom + textureSampleLevel(scene, samp, clamp(w + vec2<f32>(0.0, e), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
        bloom = bloom + textureSampleLevel(scene, samp, clamp(w - vec2<f32>(0.0, e), vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
        bloom = bloom * 0.25;
        col = col + max(bloom - vec3<f32>(0.55), vec3<f32>(0.0)) * glow * 1.6;
    }

    // SCANLINES, in WARPED space so they bend with the tube. A straight scanline
    // over a curved picture is the giveaway that the curve is a post-effect.
    //
    // Their pitch is tied to the PHYSICAL row, so they stay one-line-per-pixel-row
    // at 1080p and at 5K rather than turning into a moiré field on a dense panel.
    if (scan > 0.0) {
        let line = 0.5 + 0.5 * cos(w.y * res.y * 3.14159265);
        col = col * (1.0 - scan * 0.35 * line);
    }

    // APERTURE GRILLE — the vertical RGB stripe, again on the physical column.
    if (mask > 0.0) {
        let c = i32(floor(w.x * res.x)) % 3;
        var tint = vec3<f32>(1.0, 0.75, 0.75);
        if (c == 1) { tint = vec3<f32>(0.75, 1.0, 0.75); }
        else if (c == 2) { tint = vec3<f32>(0.75, 0.75, 1.0); }
        col = col * mix(vec3<f32>(1.0), tint, mask);
    }

    // CORNER FALLOFF, radial in the same normalised space the warp uses — so the
    // corners darken by the same amount on a 16:9 and on a 32:9.
    if (corner > 0.0) {
        let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
        let c = (w - vec2<f32>(0.5)) * a;
        let r2 = dot(c, c) / max(0.25 * dot(a, a), 0.0001);
        col = col * (1.0 - corner * 0.55 * r2);
    }

    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

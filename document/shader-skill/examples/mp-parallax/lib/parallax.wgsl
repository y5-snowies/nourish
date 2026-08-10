// THE STOCK PARALLAX, AS AN IMPORTABLE FUNCTION.
//
// Every bundle in the `Multipass` category draws this as its background, so it
// lives in one file and they `#import` it. A port of
// `two.draw/draw.vulkan/shaders/parallax.wgsl` — same hash, same noise, same
// planets, same knobs — with three things left out on purpose:
//
//   * the LOCK-SCREEN transition, which the engine drives on the built-in pass
//     and which a background pass in a graph never sees;
//   * the VIGNETTE, because a bundle that wants one applies it after compositing
//     (where it can darken the windows too, which is the point of having it);
//   * the sRGB encode, which belongs at the end of a bundle's OUTPUT pass, not
//     in the middle of its graph — encoding here would gamma the value twice.
//
// The noise hash is the driver-stable integer mix (Dave Hoskins' "hash without
// sine"), NOT `fract(sin(...))`: the sine hash diverges under range reduction at
// large coordinates on some Vulkan drivers, which showed up as rectangular boxes
// in the nebula. Keep it.
#define_import_path mp::parallax

fn hash_lane(x: f32, y: f32) -> f32 {
    let p3 = vec3<f32>(x, y, x);
    let d = dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract(((x + d) + (y + d)) * (x + d));
}

fn hash2(p: vec2<f32>) -> f32 {
    let q = fract(p * 0.1031);
    return hash_lane(q.x, q.y);
}

fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    let q = fract(vec4<f32>(i.x, i.x + 1.0, i.y, i.y + 1.0) * 0.1031);
    return mix(
        mix(hash_lane(q.x, q.z), hash_lane(q.y, q.z), f.x),
        mix(hash_lane(q.x, q.w), hash_lane(q.y, q.w), f.x),
        f.y,
    );
}

fn fbm2(p_in: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var p = p_in;
    // @optimized 2
    let octaves = 5;
    for (var i = 0; i < octaves; i = i + 1) {
        v = v + a * noise2(p);
        p = p * 2.0;
        a = a * 0.5;
    }
    return v;
}

fn draw_planet(col: vec3<f32>, uv: vec2<f32>, center: vec2<f32>, radius: f32,
               light_side: vec3<f32>, dark_side: vec3<f32>, light_dir: vec2<f32>,
               band_freq: f32) -> vec3<f32> {
    let pp = uv - center;
    // Bail on the SQUARED distance: almost every pixel is outside every planet,
    // and this way those pixels pay one dot and one compare instead of a sqrt.
    let outer = radius + 0.004;
    if (dot(pp, pp) > outer * outer) { return col; }
    let r = length(pp);
    let mask = smoothstep(0.004, -0.004, r - radius);
    let lit_dot = dot(pp, light_dir);
    let lit = smoothstep(-radius * 0.6, radius * 0.6, lit_dot);
    var base = mix(dark_side, light_side, lit);
    if (band_freq > 0.0) {
        let band = sin(pp.y * band_freq + center.x * 3.0) * 0.5 + 0.5;
        let band_noise = fbm2(pp * 15.0) * 0.15;
        base = mix(base, base * 0.75, smoothstep(0.2, 0.8, band + band_noise));
    }
    let rim = smoothstep(radius * 0.5, radius, r);
    let rim_lit = smoothstep(-radius * 0.2, radius, lit_dot);
    return mix(col, base + light_side * rim * rim_lit * 0.5, mask);
}

/// The scene, in the engine's own coordinate convention.
///
/// `frag` is the fragment's pixel position, `res` the TARGET's size — pass the
/// intermediate's size when drawing into one, not the swapchain's, or the aspect
/// is wrong by the target's scale. `pan`/`flow`/`zoom`/`time` come straight off
/// the engine push; `drift`, `stars` and `nebula` are the three knobs the stock
/// parallax exposes, and a bundle should surface them under the same names.
fn parallax_scene(frag: vec2<f32>, res: vec2<f32>, zoom: f32, time: f32,
                  pan_in: vec2<f32>, flow: vec2<f32>,
                  drift: f32, stars: f32, nebula: f32) -> vec3<f32> {
    var uv = (frag - 0.5 * res) / max(res.y, 1.0);
    uv = uv / max(zoom, 0.0001);
    // Horizontal tracks the camera as -pan; vertical is inverted here, which is
    // the baseline the per-world "Invert pan Y" toggle flips back from.
    let pan = vec2<f32>(pan_in.x, -pan_in.y);

    var col = mix(vec3<f32>(0.01, 0.015, 0.04), vec3<f32>(0.04, 0.02, 0.09), frag.y / max(res.y, 1.0));

    // Two 5-octave fbm calls are this shader's dominant cost — 40 hash
    // evaluations per pixel — and both terms scale by `nebula`, so at 0 the whole
    // block is wasted work. The branch is on a push constant: uniform, free.
    if (nebula > 0.0) {
        let neb_uv = uv * 1.5 + pan * 0.0002 + flow * 0.0003
            + vec2<f32>(time * 0.01, time * 0.005) * drift;
        let n = fbm2(neb_uv);
        let n2 = fbm2(neb_uv * 2.5 - vec2<f32>(time * 0.015, time * 0.015) * drift);
        col = col + mix(vec3<f32>(0.25, 0.05, 0.35), vec3<f32>(0.05, 0.20, 0.45), n)
            * pow(n, 1.8) * 0.5 * nebula;
        col = col + vec3<f32>(0.1, 0.3, 0.4) * (n2 * n2 * n2) * 0.25 * nebula;
    }

    // At `stars` 0 the threshold sits at 1.0, which `hash2` never exceeds — the
    // three layers would hash and discard. Skip them outright.
    if (stars > 0.0) {
        for (var i = 1; i <= 3; i = i + 1) {
            let depth = f32(i) * 0.5;
            let sp = uv * (45.0 / depth) + pan * 0.001 * depth;
            let id = floor(sp);
            let fp = fract(sp) - 0.5;
            let h = hash2(id);
            if (h > 1.0 - 0.04 * stars) {
                let twink = 0.5 + 0.5 * sin(time * 1.5 + h * 50.0);
                let dd = length(fp);
                let sc = mix(vec3<f32>(0.7, 0.9, 1.0), vec3<f32>(1.0, 0.85, 0.7), fract(h * 133.7));
                let glow = smoothstep(0.06, 0.0, dd) + smoothstep(0.2, 0.0, dd) * 0.3;
                col = col + sc * glow * twink / depth;
            }
        }
    }

    // Meteor streaks.
    {
        let d = -flow * 0.0007 + vec2<f32>(time * 0.12 * drift, 0.0);
        let p = uv * vec2<f32>(1.8, 12.0) + d;
        let id = floor(p);
        let f = fract(p) - 0.5;
        let h = hash2(id);
        if (h > 0.86) {
            let streak = smoothstep(0.5, 0.0, abs(f.y) * 5.0) * smoothstep(0.5, 0.0, abs(f.x) * 1.1);
            col = col + vec3<f32>(0.45, 0.65, 1.0) * streak * (h - 0.86) * 3.5;
        }
    }

    col = draw_planet(col, uv, vec2<f32>(-0.65, 0.30) - pan * 0.00015, 0.07,
                      vec3<f32>(0.85, 0.85, 0.90), vec3<f32>(0.18, 0.18, 0.22),
                      normalize(vec2<f32>(1.0, 0.3)), 0.0);
    col = draw_planet(col, uv, vec2<f32>(0.70, 0.15) - pan * 0.00030, 0.13,
                      vec3<f32>(0.35, 0.65, 0.55), vec3<f32>(0.08, 0.15, 0.12),
                      normalize(vec2<f32>(-0.6, 0.4)), 0.0);
    col = draw_planet(col, uv, vec2<f32>(-0.40, -0.30) - pan * 0.00055, 0.22,
                      vec3<f32>(0.90, 0.60, 0.35), vec3<f32>(0.15, 0.05, 0.08),
                      normalize(vec2<f32>(0.7, 0.5)), 15.0);
    return col;
}

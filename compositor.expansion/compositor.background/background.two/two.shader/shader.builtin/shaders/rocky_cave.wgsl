// Built-in background: "Rocky Cave" — the *inside* companion to Rocky Galaxy.
// You're deep in a cave of the rocky world, and the wall around you is riddled with
// holes that daylight pours through. The cave is a wall of faceted low-poly rock
// chunks; every opening follows their angular edges. Same quiet, unobtrusive mood.
//
// Design notes (kept deliberately restful):
//   * The dark frame is a Voronoi array of flat-shaded rock facets (a low-poly
//     look), each catching the light differently, with thin crevices between them.
//   * MANY holes are punched through that array, sitting on a jittered grid so they
//     scatter naturally (never in a row). A chunk is "open" when its facet falls
//     inside the nearest hole, so each hole is angular, not a smooth oval. Daylight
//     and a distant ridge show through, and a warm rim lights the near rocks.
//   * Each hole's size is the author's base `hole_size`, scattered per hole by a
//     noise draw (`hole_variation` controls how much the sizes wander); `hole_spacing`
//     sets how far apart the holes sit (raise it with `hole_size` for a few big ones).
//   * Dust drifts in the light. The rock frame parallaxes more than the far view.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Drift & dust" group="Cave"
// @prop dust_density float default=1.0 min=0.0 max=2.0 label="Dust density" group="Cave"
// @prop daylight float default=1.0 min=0.0 max=2.0 label="Daylight" group="Cave"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Cave"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Cave"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Cave"
// @prop hole_size float default=0.13 min=0.04 max=1.5 label="Hole size" group="Cave"
// @prop hole_variation float default=0.55 min=0.0 max=1.0 label="Hole size variation" group="Cave"
// @prop hole_spacing float default=1.0 min=0.3 max=4.0 label="Hole spacing" group="Cave"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 4>,
};
var<immediate> pc: Push;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    let uv = vec2<f32>(f32((vid << 1u) & 2u), f32(vid & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract((p3.x + p3.y) * p3.z);
}
fn hash2(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + vec3<f32>(33.33));
    return fract(vec2<f32>((p3.x + p3.y) * p3.z, (p3.x + p3.z) * p3.y));
}
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    var f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), f.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), f.x),
        f.y,
    );
}
fn fbm(p_in: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var p = p_in;
    for (var i = 0; i < 4; i = i + 1) {
        v = v + a * noise(p);
        p = p * 2.0;
        a = a * 0.5;
    }
    return v;
}

// Voronoi facet lookup: nearest jittered cell (f1), the runner-up (f2, for edges),
// the winning cell's integer coord and its centre point (in `p` space).
struct Voro { f1: f32, f2: f32, cell: vec2<f32>, center: vec2<f32> };
fn voronoi(p: vec2<f32>) -> Voro {
    let ip = floor(p);
    let fp = p - ip;
    var r: Voro;
    r.f1 = 8.0;
    r.f2 = 8.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let g = vec2<f32>(f32(x), f32(y));
            let o = g + hash2(ip + g);
            let d = length(o - fp);
            if (d < r.f1) {
                r.f2 = r.f1; r.f1 = d;
                r.cell = ip + g;
                r.center = ip + o;
            } else if (d < r.f2) {
                r.f2 = d;
            }
        }
    }
    return r;
}

// The bright world seen through the mouth: a warm daylit sky over hazy ridges.
fn exterior(p: vec2<f32>, daylight: f32) -> vec3<f32> {
    let t = clamp(p.y * 1.4 + 0.5, 0.0, 1.0);
    var e = mix(vec3<f32>(0.34, 0.24, 0.15), vec3<f32>(0.24, 0.30, 0.38), t);
    e = e + vec3<f32>(0.28, 0.19, 0.09) * exp(-((p.x - 0.12) * (p.x - 0.12) + (p.y + 0.06) * (p.y + 0.06)) * 4.0);
    let r1 = 0.02 + 0.06 * fbm(vec2<f32>(p.x * 1.4 + 4.0, 0.0));
    e = mix(e, vec3<f32>(0.24, 0.19, 0.15), smoothstep(0.01, -0.01, p.y - r1));
    let r2 = -0.10 + 0.05 * fbm(vec2<f32>(p.x * 2.3 + 9.0, 0.0));
    e = mix(e, vec3<f32>(0.16, 0.12, 0.10), smoothstep(0.01, -0.01, p.y - r2));
    return e * (0.75 + 0.45 * daylight);
}

// Holes sit on a jittered grid; `gscale` = cells per world unit (higher = more,
// tighter-spaced holes). The `hole_spacing` knob drives `gscale`, so density is
// authorable alongside "how big" and "how varied".
// Nearest hole to point `p` (world space): its centre, a stable per-hole id (for the
// size draw + jaggedness), and the distance to it. One jittered point per grid cell.
struct Hole { center: vec2<f32>, id: f32, dist: f32 };
fn nearest_hole(p: vec2<f32>, gscale: f32) -> Hole {
    let gp = p * gscale;
    let ip = floor(gp);
    let fp = gp - ip;
    var r: Hole;
    r.dist = 1e9;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let g = vec2<f32>(f32(x), f32(y));
            let o = g + 0.2 + 0.6 * hash2(ip + g);       // jitter inside the cell
            let d = length(o - fp);
            if (d < r.dist) {
                r.dist = d;
                r.center = (ip + o) / gscale;
                r.id = hash(ip + g);
            }
        }
    }
    return r;
}

// The radius of hole `id` at angle `a`: the author's base `size`, scattered per hole
// by a noise draw (`variation` = how far sizes wander), times a little angular
// jaggedness so the opening follows the rock, not a smooth oval.
fn hole_radius(a: f32, id: f32, size: f32, variation: f32) -> f32 {
    let scatter = 1.0 + variation * (fbm(vec2<f32>(id * 7.3, id * 3.1)) * 2.0 - 1.0);
    let base = size * clamp(scatter, 0.2, 2.5);
    let jag = 1.0 + 0.20 * fbm(vec2<f32>(cos(a), sin(a)) * 2.2 + id * 17.0)
            + 0.05 * sin(a * 3.0 + id * 6.283);
    return base * jag;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let frag = in.pos.xy;
    let res = pc.res_zoom_time.xy;
    let zoom = pc.res_zoom_time.z;
    let time = pc.res_zoom_time.w;
    let pan_in = pc.pan_flow.xy;
    let lock_amount = pc.lock_alpha.x;
    let alpha = pc.lock_alpha.y;

    let drift = pc.params[0].x;
    let dust_density = pc.params[0].y;
    let daylight = pc.params[0].z;
    let vignette = pc.params[0].w;
    let vig_radius = pc.params[1].x;
    let vig_softness = pc.params[1].y;
    let hole_size = pc.params[1].z;
    let hole_variation = pc.params[1].w;
    let hole_spacing = pc.params[2].x;
    // Cells per world unit: raising hole_spacing widens the grid → fewer, farther holes.
    let hole_grid = 2.4 / max(hole_spacing, 0.05);

    var uv = (frag - 0.5 * res) / res.y;
    let screen_uv = uv;
    uv = uv / zoom;
    // Pan convention: on-screen content tracks the camera as -pan_in on both axes.
    // This scene anchors content as `uv - pan`, so both axes are negated to land
    // on that shared convention (the other scenes reach it as `+pan_in`).
    let pan = vec2<f32>(-pan_in.x, -pan_in.y);

    let world = uv - pan * 0.0006;
    // Rock-facet field in the (near-parallaxed) frame space.
    let scale = 5.5;
    let fp = world * scale;
    let v = voronoi(fp);

    // Is this facet part of a hole? Test the facet's CENTRE against the nearest
    // hole, so every opening follows the angular rock edges (not a smooth oval).
    let fc = v.center / scale;                         // this facet's centre (world)
    let ch = nearest_hole(fc, hole_grid);
    let crel = fc - ch.center;
    let crad = length(crel * vec2<f32>(1.0, 1.18));
    let chr = hole_radius(atan2(crel.y, crel.x), ch.id, hole_size, hole_variation);
    let is_open = crad < chr;

    var col: vec3<f32>;
    if (is_open) {
        // Daylight through the hole (far parallax).
        col = exterior(uv * 0.85 - pan * 0.00018, daylight);
    } else {
        // Flat-shaded low-poly rock facet: a constant pseudo-normal per cell lit by
        // a fixed key, so each chunk catches a different flat tone.
        let nrm = normalize(vec3<f32>((hash2(v.cell) - 0.5) * 1.6, 0.85));
        let key = normalize(vec3<f32>(-0.45, 0.55, 0.55));
        let sh = clamp(dot(nrm, key), 0.0, 1.0);
        var rock = mix(vec3<f32>(0.014, 0.011, 0.010), vec3<f32>(0.075, 0.06, 0.05), sh * sh);
        // Thin dark crevices between facets (Voronoi edges).
        let edge = smoothstep(0.0, 0.05, v.f2 - v.f1);
        rock = rock * (0.35 + 0.65 * edge);
        // Warm rim: facets hugging a hole edge catch a little spill light.
        let rim = smoothstep(0.2, 0.0, crad - chr);
        rock = rock + vec3<f32>(0.32, 0.21, 0.10) * rim * rim * (0.45 + 0.4 * daylight);
        col = rock;
    }

    // Soft shafts of light reaching in from the holes (per-fragment nearest hole).
    let fh = nearest_hole(world, hole_grid);
    let hrel = world - fh.center;
    let hrad = length(hrel * vec2<f32>(1.0, 1.18));
    let hr = hole_radius(atan2(hrel.y, hrel.x), fh.id, hole_size, hole_variation);
    col = col + vec3<f32>(0.12, 0.08, 0.04) * exp(-(hrad - hr) * 3.0) * f32(!is_open) * smoothstep(hr + 0.5, hr, hrad) * (0.4 + 0.4 * daylight);

    // Dust motes drifting in the light near the mouth.
    for (var i = 0; i < 2; i = i + 1) {
        let depth = 1.0 + f32(i) * 0.7;
        let sp = vec2<f32>(uv.x * (10.0 / depth) + pan.x * 0.0011 * depth + time * 0.03 * drift / depth,
                           uv.y * (10.0 / depth) - time * 0.02 * drift / depth + pan.y * 0.0011 * depth);
        let id = floor(sp);
        let f = fract(sp) - 0.5;
        let h = hash(id + f32(i) * 29.0);
        if (h > 1.0 - 0.05 * dust_density) {
            let dd = length(f);
            let near_light = smoothstep(hr + 0.6, hr - 0.1, hrad);
            col = col + vec3<f32>(0.5, 0.36, 0.2) * smoothstep(0.07, 0.0, dd) * (0.5 + 0.5 * sin(time + h * 30.0)) * near_light / (depth * 3.0);
        }
    }

    // Lock-screen ease: let the daylight dim toward dusk.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.008, 0.006, 0.005), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

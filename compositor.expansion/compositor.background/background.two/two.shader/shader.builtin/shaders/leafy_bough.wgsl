// Built-in background: "Leafy Bough" — the woody companion to Rocky Cave and Fiery
// Cavern. You're tucked inside a hollow tree branch: a wall of low-poly bark chunks,
// riddled with knot-holes that open onto dappled green canopy light. Same faceted,
// holed construction as the cave scenes, lit by soft forest daylight.
//
// Design notes (kept deliberately restful):
//   * The wall is a Voronoi array of flat-shaded bark facets (a low-poly look), with
//     a fine wood grain drawn through them and thin dark crevices between chunks.
//   * MANY knot-holes are punched through the wall on a jittered grid, each opening
//     onto a dappled canopy — layered greens with warm sun breaking through the
//     leaves. A chunk is "open" when its facet falls inside the nearest hole, so
//     every hole is angular, following the bark edges — not a smooth oval.
//   * Each hole's size is `hole_size`, scattered per hole by a noise draw
//     (`hole_variation`); `hole_spacing` sets how far apart the holes sit (raise it
//     with `hole_size` for a few big openings).
//   * Pollen motes drift in the light. The bark wall parallaxes more than the canopy.
//
// Author-exposed knobs (parsed from the `@prop` lines below → params slots):
// @prop drift_speed float default=1.0 min=0.0 max=6.0 label="Sway & motes" group="Bough"
// @prop mote_density float default=1.0 min=0.0 max=2.0 label="Pollen density" group="Bough"
// @prop daylight float default=1.0 min=0.0 max=2.0 label="Canopy light" group="Bough"
// @prop vignette float default=0.0 min=0.0 max=1.0 label="Vignette amount" group="Bough"
// @prop vignette_radius float default=1.12 min=0.5 max=2.0 label="Vignette radius" group="Bough"
// @prop vignette_softness float default=0.6 min=0.05 max=2.0 label="Vignette softness" group="Bough"
// @prop hole_size float default=0.14 min=0.04 max=1.5 label="Knot-hole size" group="Bough"
// @prop hole_variation float default=0.55 min=0.0 max=1.0 label="Hole size variation" group="Bough"
// @prop hole_spacing float default=1.0 min=0.3 max=4.0 label="Hole spacing" group="Bough"

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

// The dappled canopy seen through a knot-hole: layered forest green with warm sun
// breaking through gaps in the leaves. `p` is world space; `time` sways it gently.
fn canopy(p: vec2<f32>, daylight: f32, time: f32) -> vec3<f32> {
    let sway = vec2<f32>(sin(time * 0.25) * 0.06, time * 0.02);
    let leaves = fbm(p * 3.0 + sway);
    let dapple = fbm(p * 7.0 - sway * 1.4);
    // Shaded canopy depths → sunlit leaf.
    var c = mix(vec3<f32>(0.05, 0.09, 0.035), vec3<f32>(0.20, 0.33, 0.12), smoothstep(0.35, 0.65, leaves));
    // Warm light spilling through gaps in the leaves, with brighter hotspots.
    let sun = smoothstep(0.60, 0.80, dapple);
    c = mix(c, vec3<f32>(0.80, 0.82, 0.48), sun);
    c = c + vec3<f32>(0.35, 0.34, 0.16) * smoothstep(0.82, 0.95, dapple);
    return c * (0.55 + 0.6 * daylight);
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
// jaggedness so the opening follows the bark, not a smooth oval.
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
    let mote_density = pc.params[0].y;
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
    // Bark-facet field in the (near-parallaxed) frame space.
    let scale = 5.5;
    let fp = world * scale;
    let v = voronoi(fp);

    // Is this facet a knot-hole? Test the facet's CENTRE against the nearest hole,
    // so every opening follows the angular bark edges (not a smooth oval).
    let fc = v.center / scale;                         // this facet's centre (world)
    let ch = nearest_hole(fc, hole_grid);
    let crel = fc - ch.center;
    let crad = length(crel * vec2<f32>(1.0, 1.18));
    let chr = hole_radius(atan2(crel.y, crel.x), ch.id, hole_size, hole_variation);
    let is_open = crad < chr;

    var col: vec3<f32>;
    if (is_open) {
        // Dappled canopy through the knot-hole (far, slow parallax).
        col = canopy(uv * 0.9 - pan * 0.00018, daylight, time * drift);
    } else {
        // Flat-shaded low-poly bark facet: a constant pseudo-normal per cell lit by
        // a fixed key, so each chunk catches a different flat tone.
        let nrm = normalize(vec3<f32>((hash2(v.cell) - 0.5) * 1.6, 0.85));
        let key = normalize(vec3<f32>(-0.45, 0.55, 0.55));
        let sh = clamp(dot(nrm, key), 0.0, 1.0);
        var wood = mix(vec3<f32>(0.055, 0.036, 0.022), vec3<f32>(0.17, 0.115, 0.070), sh * sh);
        // Wood grain: fine noise stretched along the branch, varying within a facet.
        let grain = fbm(vec2<f32>(world.x * 3.0, world.y * 26.0));
        wood = wood * (0.82 + 0.32 * grain);
        // Thin dark crevices between the bark chunks (Voronoi edges).
        let edge = smoothstep(0.0, 0.05, v.f2 - v.f1);
        wood = wood * (0.4 + 0.6 * edge);
        // Green-gold rim: bark hugging a knot-hole catches canopy spill light.
        let rim = smoothstep(0.22, 0.0, crad - chr);
        wood = wood + vec3<f32>(0.28, 0.30, 0.14) * rim * rim * (0.4 + 0.4 * daylight);
        col = wood;
    }

    // Soft shafts of canopy light reaching in from the holes (per-fragment nearest hole).
    let fh = nearest_hole(world, hole_grid);
    let hrel = world - fh.center;
    let hrad = length(hrel * vec2<f32>(1.0, 1.18));
    let hr = hole_radius(atan2(hrel.y, hrel.x), fh.id, hole_size, hole_variation);
    col = col + vec3<f32>(0.10, 0.11, 0.05) * exp(-(hrad - hr) * 3.0) * f32(!is_open) * smoothstep(hr + 0.5, hr, hrad) * (0.4 + 0.4 * daylight);

    // Pollen motes drifting in the light near the knot-holes.
    for (var i = 0; i < 2; i = i + 1) {
        let depth = 1.0 + f32(i) * 0.7;
        let sp = vec2<f32>(uv.x * (10.0 / depth) + pan.x * 0.0011 * depth + time * 0.02 * drift / depth,
                           uv.y * (10.0 / depth) - time * 0.015 * drift / depth + pan.y * 0.0011 * depth);
        let id = floor(sp);
        let f = fract(sp) - 0.5;
        let h = hash(id + f32(i) * 29.0);
        if (h > 1.0 - 0.05 * mote_density) {
            let dd = length(f);
            let near_light = smoothstep(hr + 0.6, hr - 0.1, hrad);
            col = col + vec3<f32>(0.5, 0.5, 0.28) * smoothstep(0.06, 0.0, dd) * (0.5 + 0.5 * sin(time + h * 30.0)) * near_light / (depth * 3.0);
        }
    }

    // Lock-screen ease: let the canopy dim toward a still, dusky green.
    var l = clamp(lock_amount, 0.0, 1.0);
    l = l * l * (3.0 - 2.0 * l);
    col = mix(col, col * 0.5 + vec3<f32>(0.006, 0.009, 0.006), l);

    let vig = smoothstep(vig_radius, vig_radius - vig_softness, length(screen_uv));
    col = col * mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    // Per-world sRGB flag (push lock_alpha.z): gamma-encode for the brighter,
    // preview-matching look on a non-sRGB scanout buffer. Off = raw values.
    let outc = select(col, pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)), pc.lock_alpha.z > 0.5);
    return vec4<f32>(outc, 1.0) * (alpha * 0.75);
}

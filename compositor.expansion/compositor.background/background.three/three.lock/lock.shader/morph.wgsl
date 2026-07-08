// Sphere↔plane morph shader (sphere stored, plane computed).
//
// The mesh stores sphere positions and outward normals as attributes.
// At `flatness = 0` the geometry renders as the stored sphere.
// At `flatness = 1` the vertex shader flattens it to the XY plane using
// each vertex's UV.
//
// Three independent band flatness values drive a staggered mechanical
// fold: outer band flips first, middle next, inner last.
//
// Fragment shader applies Lambertian lighting using the stored sphere
// normal (slight inaccuracy during mid-morph since the actual surface
// normal would be the interpolated derivative, but the eye doesn't catch
// this on a fast transition).

#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_clip}

struct MorphParams {
    t: f32,
    going_to_sphere: f32,
    plane_aspect: f32,
    sphere_radius: f32,
    light_dir_x: f32,
    light_dir_y: f32,
    light_dir_z: f32,
    light_intensity: f32,
    ambient_intensity: f32,
    ready: f32,
    _pad1: f32,
    _pad2: f32,
};
@group(3) @binding(0) var<uniform> params: MorphParams;
@group(3) @binding(1) var snapshot_tex: texture_2d<f32>;
@group(3) @binding(2) var snapshot_sampler: sampler;

struct VertexInput {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,    // sphere position
    @location(1) normal: vec3<f32>,      // sphere normal
    @location(2) uv: vec2<f32>,          // plane position (UV_0)
    @location(3) tex_uv: vec2<f32>,      // texture coords (UV_1)
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_uv: vec2<f32>,
    @location(1) world_normal: vec3<f32>,
};

const RADIAL_MAX: f32 = 0.7071068;
const N_BANDS: f32 = 100.0;
// EXPERIMENT KNOBS -----------------------------------------------------------
// BAND_OVERLAP: how many stagger-widths each band's fold spans. Larger = more
//   bands folding at once = a wide, smooth, obviously-curving wrap instead of a
//   thin sweeping crease. (was 2.5)
const BAND_OVERLAP: f32 = 6.0;
// WRAP_FROM_EDGE: true  -> outer edge/poles fold FIRST, camera-facing center
//   folds LAST (the flat image visibly wraps inward — try this first).
//                 false -> original center-first behaviour.
const WRAP_FROM_EDGE: bool = true;
// ----------------------------------------------------------------------------

//fn plane_pos(uv: vec2<f32>, aspect: f32) -> vec3<f32> {
//    return vec3<f32>((uv.x - 0.5) * aspect, (uv.y - 0.5), 0.0);
//}
fn plane_pos(uv: vec2<f32>, aspect: f32) -> vec3<f32> {
    return vec3<f32>((uv.x - 0.5) * aspect, (uv.y - 0.5), 0.3);  // pushed forward
}
//fn band_flatness(uv: vec2<f32>) -> f32 {
//    let d = length(uv - vec2<f32>(0.5, 0.5)) / RADIAL_MAX;
//    let band_index = floor(d * N_BANDS);
//    let stagger = 1.0 / N_BANDS;
//    let band_duration = stagger * BAND_OVERLAP;
//
//    // Morph (going_to_sphere = 1): outer band index N_BANDS-1 starts at t=0.
//    // Unmorph (going_to_sphere = 0): inner band index 0 starts at t=0.
//    let outer_first_start = 1.0 - (band_index + 1.0) * stagger;
//    let inner_first_start = band_index * stagger;
//    let band_start = mix(inner_first_start, outer_first_start, params.going_to_sphere);
//
//    let local_t = clamp((params.t - band_start) / band_duration, 0.0, 1.0);
//
//    // local_t = 0 → at the band's starting state (plane if morphing, sphere if unmorphing)
//    // local_t = 1 → at the band's ending state (sphere if morphing, plane if unmorphing)
//    // flatness convention: 0 = sphere, 1 = plane
//    return mix(local_t, 1.0 - local_t, params.going_to_sphere);
//}

fn band_flatness(uv: vec2<f32>) -> f32 {
    let d = length(uv - vec2<f32>(0.5, 0.5)) / RADIAL_MAX;
    let band_index = floor(d * N_BANDS);
    let stagger = 1.0 / N_BANDS;
    let band_duration = stagger * BAND_OVERLAP;

    // Order the fold. WRAP_FROM_EDGE flips which end goes first: the map centre
    // (band_index 0) is the sphere's front pole — the point facing the camera —
    // so edge-first makes the visible surface morph LAST and spreads the wrap
    // across the whole animation instead of resolving it off-screen at the back.
    // The min() cap keeps the last band finishing by t=1 so no vertex is left
    // with residual flatness (spikes) at the resting sphere.
    let ordered = select(band_index, N_BANDS - band_index, WRAP_FROM_EDGE);
    let band_start = min(ordered * stagger, 1.0 - band_duration);

    let local_t = clamp((params.t - band_start) / band_duration, 0.0, 1.0);
    return mix(local_t, 1.0 - local_t, params.going_to_sphere);
}

fn local_flatness_unmorph(uv: vec2<f32>, t: f32) -> f32 {
    let d = length(uv - vec2<f32>(0.5, 0.5)) / RADIAL_MAX;
    let band_index = floor(d * N_BANDS);
    
    let stagger = 1.0 / N_BANDS;
    let band_start = band_index * stagger;  // inner band starts at 0
    let band_duration = stagger * BAND_OVERLAP;
    
    let local_t = clamp((t - band_start) / band_duration, 0.0, 1.0);
    return local_t;  // 0 → 1 (sphere → plane)
}
fn local_flatness(uv: vec2<f32>, t: f32) -> f32 {
    let d = length(uv - vec2<f32>(0.5, 0.5)) / RADIAL_MAX;
    let band_index = floor(d * N_BANDS);  // 0 (center) to N_BANDS-1 (corners)
    
    // For morph (outer first): outer band index is high, starts first.
    // Band's start time as fraction of total: 1 - (band_index+1)/N_BANDS
    let stagger = 1.0 / N_BANDS;
    let band_start = 1.0 - (band_index + 1.0) * stagger;
    let band_duration = stagger * BAND_OVERLAP;
    
    let local_t = clamp((t - band_start) / band_duration, 0.0, 1.0);
    
    // local_t goes 0→1 as the band animates. We want flatness 1→0.
    return 1.0 - local_t;
}

//@vertex
//fn vertex(in: VertexInput) -> VertexOutput {
//    let p_sphere = in.position;
//    let p_plane = plane_pos(in.uv, params.plane_aspect);
//    let f = band_flatness(in.uv);
//    let p = mix(p_sphere, p_plane, f);
//
//    let model = get_world_from_local(in.instance_index);
//
//    let world_normal_v4 = model * vec4<f32>(in.normal, 0.0);
//    let world_normal = normalize(world_normal_v4.xyz);
//
//    var out: VertexOutput;
//    out.clip_position = mesh_position_local_to_clip(model, vec4<f32>(p, 1.0));
//    out.uv = in.uv;
//    out.world_normal = world_normal;
//    return out;
//}

const PLANE_LIMIT_KAPPA: f32 = 0.001;
fn cap_position(uv: vec2<f32>, flatness: f32, aspect: f32) -> vec3<f32> {
    let xy = (uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(aspect, 1.0);
    let xy_radial = length(xy);
    
    let xy_radial_max = sqrt(aspect * aspect * 0.25 + 0.25);
    let kappa_max = 3.14159 / xy_radial_max;
    let kappa = kappa_max * (1.0 - flatness);
    
    if (kappa < 0.001) {
        return vec3<f32>(xy.x, xy.y, 0.0);
    }
    
    let r = 1.0 / kappa;
    let theta = xy_radial * kappa;
    let sin_theta = sin(theta);
    let cos_theta = cos(theta);
    
    let xy_safe = xy / max(xy_radial, 0.00001);
    let cos_phi = xy_safe.x;
    let sin_phi = xy_safe.y;
    
    return vec3<f32>(
        r * sin_theta * cos_phi,
        r * sin_theta * sin_phi,
        -(r * cos_theta - r),   // ← negated: now bulges toward +Z
    );
}

fn cap_normal(uv: vec2<f32>, flatness: f32, aspect: f32) -> vec3<f32> {
    let xy_radial_max = sqrt(aspect * aspect * 0.25 + 0.25);
    let kappa_max = 3.14159 / xy_radial_max;
    let kappa = kappa_max * (1.0 - flatness);
    
    if (kappa < 0.001) {
        return vec3<f32>(0.0, 0.0, 1.0);
    }
    
    let r = 1.0 / kappa;
    let pos = cap_position(uv, flatness, aspect);
    // Cap center is now at (0, 0, +r), behind the front pole from camera's view.
    let center = vec3<f32>(0.0, 0.0, r);
    return normalize(pos - center);
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    let p_sphere = in.position;
    let p_plane = vec3<f32>(in.uv.x, in.uv.y, 0.0);  // plane is at z=0
    
    let f = band_flatness(in.tex_uv);  // band logic uses tex_uv (lng/lat normalized)
    let p = mix(p_sphere, p_plane, f);
    
    // Normal: for now use sphere normal throughout. At flatness=1 the surface
    // is flat and the "true" normal is (0,0,1) — slight inaccuracy at full
    // plane. Improvement: mix(sphere_normal, plane_normal, f).
    let plane_normal = vec3<f32>(0.0, 0.0, 1.0);
    let local_normal = normalize(mix(in.normal, plane_normal, f));
    
    let model = get_world_from_local(in.instance_index);
    let world_normal_v4 = model * vec4<f32>(local_normal, 0.0);
    let world_normal = normalize(world_normal_v4.xyz);
    
    var out: VertexOutput;
    out.clip_position = mesh_position_local_to_clip(model, vec4<f32>(p, 1.0));
    out.tex_uv = in.tex_uv;
    out.world_normal = world_normal;
    return out;
}
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // tex_uv already matches the snapshot's orientation (lat_t = 0 at the top
    // row); the previous `1.0 - y` inverted it, which showed the capture
    // upside-down on the sphere.
    let uv = in.tex_uv;
    let tex_color = textureSample(snapshot_tex, snapshot_sampler, uv);

    // Fully transparent before the cast is bridged in, so the plane never flashes
    // a placeholder gradient over the background (the fold is also gated on the
    // cast being ready — see PreMorphDelay). Once the cast lands it's opaque.
    let fallback = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    let use_texture = step(0.01, tex_color.a);
    let base_color = mix(fallback, tex_color, use_texture);

    let light_dir = normalize(vec3<f32>(
        params.light_dir_x,
        params.light_dir_y,
        params.light_dir_z,
    ));
    let n = normalize(in.world_normal);

    // Half-Lambert: maps dot from [-1, 1] to [0, 1], giving smooth
    // gradient that avoids harsh shadow boundaries. Combined with
    // ambient, back-faces still get some illumination.
    let half_lambert = dot(n, light_dir) * 0.5 + 0.5;

    let intensity = params.ambient_intensity + params.light_intensity * half_lambert;

    // `ready` gates the first (uniform-lagged) frames of a freshly-created
    // material to fully transparent, so no default-uniform sphere/blank flashes.
    return vec4<f32>(base_color.rgb * intensity, base_color.a * params.ready);
}
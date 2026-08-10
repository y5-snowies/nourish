// TB — `storage`, pass 3 of 3: turn the light field into embers.
//
// Reads the grid `simulate` just filled and adds it over the desktop. This pass
// never sees a particle — the grid is the whole interface between the simulation
// and the picture, which is what keeps the cost independent of how many embers
// there are.
//
// BILINEAR, and no more. The spread is done on the WRITE side (`simulate` splats
// each ember over a 3×3 neighbourhood with normalised weights), so all that is
// left here is to reconstruct a smooth value between cell centres. Bilinear is
// exactly that, and its weights sum to one everywhere, so there is no pulsing as
// a pixel crosses a cell boundary.
//
// This is the read a storage IMAGE would do in fixed-function hardware. It is
// four loads and three `mix`es — worth knowing, because "the grid is visible"
// is a write-side problem and moving the field into an image would not have
// fixed it. What an image buys here is the ALU, not the smoothness.
//
// @prop glow    float default=1.0  min=0.0 max=4.0 step=0.01 label="Glow"        group="Embers"
// @prop warm    float default=0.65 min=0.0 max=1.0 step=0.01 label="Warmth"      group="Embers"
// @prop dim     float default=0.15 min=0.0 max=1.0 step=0.01 label="Dim desktop" group="Embers"

const GRID_W: i32 = 320;
const GRID_H: i32 = 180;
const UNIT: f32 = 4096.0;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

struct Field { cell: array<atomic<u32>, 57600>, };
@group(2) @binding(0) var<storage, read_write> field: Field;

/// One cell, clamped at the edges so the border does not darken.
fn cell(x: i32, y: i32) -> f32 {
    let cx = clamp(x, 0, GRID_W - 1);
    let cy = clamp(y, 0, GRID_H - 1);
    return f32(atomicLoad(&field.cell[cy * GRID_W + cx])) / UNIT;
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let p_glow = pc.params[0].x;
    let p_warm = pc.params[0].y;
    let p_dim = pc.params[0].z;

    var col = textureSampleLevel(scene, samp, uv, 0.0).rgb * (1.0 - p_dim);

    // Cell space, offset by half a cell so `base` is the cell whose CENTRE is up
    // and to the left of this pixel — the standard bilinear setup, and the same
    // offset `simulate` splats with, so write and read agree about where a cell is.
    let g = uv * vec2<f32>(f32(GRID_W), f32(GRID_H)) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(g));
    let f = g - floor(g);
    let top = mix(cell(base.x, base.y), cell(base.x + 1, base.y), f.x);
    let bot = mix(cell(base.x, base.y + 1), cell(base.x + 1, base.y + 1), f.x);
    let lit = mix(top, bot, f.y);

    // Ember colour: hot core toward white, edges toward amber.
    let heat = clamp(lit * p_glow * 0.9, 0.0, 4.0);
    let tint = mix(vec3<f32>(0.55, 0.75, 1.0), vec3<f32>(1.0, 0.55, 0.18), p_warm);
    col = col + tint * heat + vec3<f32>(1.0) * max(heat - 1.0, 0.0) * 0.5;

    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

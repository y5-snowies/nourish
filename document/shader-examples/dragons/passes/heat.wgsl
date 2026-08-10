// dragons, pass 1 — publish the fire field.
//
// The world's fire lives in a storage buffer that the band pass deposits into
// wherever it actually draws flame (see `band.wgsl`). This pass reads that
// buffer, decays it, lets it spread, and writes the result to an ordinary
// low-resolution target — so everything downstream can just SAMPLE the fire,
// bilinearly filtered, instead of hand-rolling grid lookups.
//
// It runs before the band, so it publishes the field as of the end of the last
// frame. One frame of lag on a fire that lingers for seconds is not visible.
//
// This pass never WRITES the buffer, only reads it. All the writing happens in
// one place, from atomics, which is what keeps the simulation free of races: an
// update that read neighbours and wrote its own cell in the same pass would see
// a different arbitrary subset of its neighbours' updates every frame, and
// would still look plausible while being wrong.

#import dragons::common::GRID_W
#import dragons::common::GRID_H
#import dragons::common::GRID_CELLS
#import dragons::common::heat_cell
#import dragons::common::heat_now

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 1>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;

struct Field {
    cell: array<atomic<u32>, 82944>,
};
@group(2) @binding(0) var<storage, read_write> field: Field;

fn heat_at(uv: vec2<f32>, t: f32) -> f32 {
    return heat_now(atomicLoad(&field.cell[heat_cell(uv)]), t);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let t = pc.res_zoom_time.w;
    let uv = frag.xy / res;
    let step = vec2<f32>(1.0 / GRID_W, 1.0 / GRID_H);

    var h = heat_at(uv, t);
    // Spread sideways a little, and upward strongly: heat rises, so a cell
    // takes most from the cell BELOW it (screen y grows downward).
    h = max(h, heat_at(uv + vec2<f32>(step.x, 0.0), t) * 0.70);
    h = max(h, heat_at(uv - vec2<f32>(step.x, 0.0), t) * 0.70);
    h = max(h, heat_at(uv + vec2<f32>(0.0, step.y), t) * 0.88);
    h = max(h, heat_at(uv - vec2<f32>(0.0, step.y), t) * 0.55);
    h = max(h, heat_at(uv + vec2<f32>(0.0, step.y * 2.0), t) * 0.62);

    return vec4<f32>(h, 0.0, 0.0, 1.0);
}

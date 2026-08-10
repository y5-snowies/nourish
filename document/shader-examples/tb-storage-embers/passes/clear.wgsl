// TB — `storage`, pass 1 of 3: zero the light field.
//
// The FIELD is per-frame and must start empty; the PARTICLES are not and must
// not be touched. That split is the whole reason this bundle declares two
// buffers instead of one: storage survives the frame, and the two halves of the
// state want opposite things from that.
//
// A separate pass because invocations within one draw have no ordering relative
// to each other — clearing and splatting together would clear some cells after
// they had already been added to, and the field would flicker in a pattern that
// looks like noise rather than like a bug. The engine emits a storage barrier
// after every intermediate pass, so `simulate` sees a fully cleared field.
//
// STRIDED, so the clear does not depend on the output size. A fixed
// one-invocation-per-cell mapping silently stops clearing the tail of the field
// on a small display, and cells that are never cleared accumulate forever — a
// permanent bright smear in the corner of the screen with nothing to explain it.
//
// It writes to `scratch` because a pass must write somewhere; the buffer is the
// real output, which is why `scratch` is quarter-scale and nobody samples it.

const CELLS: u32 = 57600u;

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

struct Field { cell: array<atomic<u32>, 57600>, };
@group(2) @binding(0) var<storage, read_write> field: Field;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let px = vec2<u32>(frag.xy);
    let w = u32(res.x);
    let stride = max(w * u32(res.y), 1u);
    for (var i = px.y * w + px.x; i < CELLS; i = i + stride) {
        atomicStore(&field.cell[i], 0u);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

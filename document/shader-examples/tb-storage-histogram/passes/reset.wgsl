// TB — `storage`, pass 1 of 3: zero the tally.
//
// A separate pass, and that is the lesson. Resetting and accumulating in ONE pass
// does not work: invocations within a single draw have no ordering relative to
// each other, so some pixels would add before the clear and some after, and the
// histogram would be a different arbitrary subset every frame. It would still
// look like a histogram — which is exactly why this is worth a bundle.
//
// The engine emits a storage barrier after every intermediate pass, so by the time
// `tally` runs, every store here is visible to it.
//
// The engine zeroes a storage buffer ONCE, when it allocates it. Everything after
// that is the bundle's business: storage survives the frame, which is what an
// accumulator wants and what a per-frame tally must undo itself.
//
// It writes to `scratch` because a pass must write SOMETHING — the buffer is the
// real output and the target is ignored, which is why `scratch` is quarter-scale
// and never sampled by anyone.

const BINS: u32 = 64u;

struct Bins { count: array<atomic<u32>, 256>, };
@group(2) @binding(0) var<storage, read_write> bins: Bins;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    // One invocation per bin, chosen by pixel coordinate: exactly 64 of this
    // pass's pixels take the job, no bin is claimed twice and none is missed.
    // `scratch` is quarter-scale, so its narrowest sensible width still far
    // exceeds BINS.
    let px = vec2<u32>(frag.xy);
    if (px.y == 0u && px.x < BINS) {
        atomicStore(&bins.count[px.x], 0u);
    }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

// AN ANIMATING WARP — the case a static bake cannot serve.
//
// The curvature breathes, so `hit_inverse` takes the optional fourth argument:
// the clock the pass was pushed for. That makes it a different kind of function —
// a field valid for one instant — and the engine treats it as one:
//
//   * `evaluate: "map_static"` is REFUSED at load. A bake of one instant is wrong
//     every frame after it, silently.
//   * `evaluate: "map"` renders this on the GPU as part of each frame and reads
//     the grid back, one frame behind. 16k fragments is nothing there; 16k
//     interpreted evaluations per frame on the CPU would be about a millisecond
//     of a sixteen.
//   * `evaluate: "pointwise"` would also be correct, and for a warp this cheap it
//     is genuinely the better answer. This bundle exists to exercise the GPU
//     producer, not to argue it is optimal here.
//
// Still PURE, and still one definition: the render pass `#import`s this exact
// function, and so does the generated grid pass the engine composes.
#define_import_path crt_live::warp

fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>, t: f32) -> vec2<f32> {
    // The curve breathes around its declared value. Nothing else changes, so any
    // pointer error you can see is the map's age or its interpolation — not a
    // different effect.
    let k = params.x * (1.0 + 0.5 * sin(t * 0.9));
    let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let c = (uv - vec2<f32>(0.5, 0.5)) * a;
    let r2 = dot(c, c);
    return vec2<f32>(0.5, 0.5) + c * (1.0 + k * r2) / a;
}

// The bundle's displacement, in ONE place.
//
// `hit_inverse` answers "screen point -> where did this content come from". A
// fragment shader that samples through it and a pointer that is corrected by it
// are asking the same question, so they must not be two functions. The render
// pass `#import`s this; the engine parses this same file and interprets it on the
// CPU (`shader.hit`). Change the curve here and the cursor follows, with nothing
// else to update.
//
// PURE, BY CONTRACT. No bindings, no textures, no globals — everything arrives as
// an argument. That is what lets the host evaluate it at all; `shader.hit`
// refuses the bundle at load if this drifts out of the subset, naming what broke.
#define_import_path crt_spin::warp

fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    let k = params.x;
    // Aspect-corrected so the curve is round on a wide display rather than an
    // ellipse — and corrected symmetrically, so the inverse stays this same
    // expression rather than acquiring a special case.
    let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let c = (uv - vec2<f32>(0.5, 0.5)) * a;
    let r2 = dot(c, c);
    return vec2<f32>(0.5, 0.5) + c * (1.0 + k * r2) / a;
}

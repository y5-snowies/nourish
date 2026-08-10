// The bundle's displacement, in ONE place — identical maths to `tb-crt-spin`.
//
// The two bundles differ in nothing but `hit.evaluate`: this one is served from a
// BAKED MAP, that one POINTWISE by the interpreter. Keeping the function identical
// is the point — any difference you can see between them is the map's
// interpolation error, not a different effect.
//
// PURE, BY CONTRACT. No bindings, no textures, no globals; everything arrives as
// an argument. That purity is exactly what makes it griddable: a function of
// position alone has the same answer every time it is asked, so asking it 128×128
// times and interpolating is equivalent to asking it per event — to within
// curvature × cell size, which at cursor scale is nothing.
#define_import_path crt_map::warp

fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    let k = params.x;
    let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let c = (uv - vec2<f32>(0.5, 0.5)) * a;
    let r2 = dot(c, c);
    return vec2<f32>(0.5, 0.5) + c * (1.0 + k * r2) / a;
}

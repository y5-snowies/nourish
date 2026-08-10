// Shared color helpers, imported by the bloom passes via naga_oil `#import`.
// The `#define_import_path` line names the module; passes `#import color::<fn>`.
#define_import_path color

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Reinhard tone map with exposure — keeps the additive bloom from clipping.
fn tonemap(c: vec3<f32>, exposure: f32) -> vec3<f32> {
    let x = c * exposure;
    return x / (x + vec3<f32>(1.0));
}

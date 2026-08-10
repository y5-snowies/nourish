// Intentionally BROKEN WGSL — a compile-error test. The loader logs the naga
// parse error and falls back to the built-in parallax (it never crashes).
// There is also no vertex entry point, a second reason compilation fails.
//
// @prop foo float default=1.0

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0   // <-- missing closing paren + semicolon
}

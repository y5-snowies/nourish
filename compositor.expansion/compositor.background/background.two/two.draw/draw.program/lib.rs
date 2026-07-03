//! Compiles the GLES pixel-shader program for the parallax background.
//
// Mountain: Clouds are just squares. Could be a more detailed square. Like in
// LowPoly games. Similarly the mountains can be better on that regard. Improve
// on the colors too. Feature: Time: the clouds should always move.
// Space Station: I actually meant a grounded space station. Perhaps with
// something ready to launch. Has brown and bluish purple touches.
// Feature: Time: whatever flows should always move.
// Space: We can keep current space. But with some changes:
// 1. Add a couple of more planets.
// 2. Remove the current thing that looks like a marker at the center- I think
//    it is meant to represent a station.
// 3. Feature: Time: A rocket that always seem to be moving upwards. After some
//    time, it'll reloop to the start. It has visible ignition engine.

use compositor_developer_debug_instance_record::abort;
use smithay::backend::renderer::gles::{
    GlesError, GlesPixelProgram, GlesRenderer, UniformName, UniformType,
};

/// The engine uniforms every parallax-background GLES shader receives. A custom
/// `gles/shader.frag` declares (a subset of) these; they carry the pan/zoom/time/
/// flow/lock state the built-in `spacev3.frag` uses.
fn engine_uniforms() -> [UniformName<'static>; 11] {
    [
        UniformName::new("u_time", UniformType::_1f),
        UniformName::new("u_lock_amount", UniformType::_1f),
        UniformName::new("u_pan", UniformType::_2f),
        UniformName::new("u_flow_offset", UniformType::_2f),
        UniformName::new("pan_velocity", UniformType::_2f),
        UniformName::new("u_zoom", UniformType::_1f),
        UniformName::new("u_resolution", UniformType::_2f),
        // Shader-authored `@prop` values, four vec4 slots (16 floats).
        UniformName::new("u_param0", UniformType::_4f),
        UniformName::new("u_param1", UniformType::_4f),
        UniformName::new("u_param2", UniformType::_4f),
        UniformName::new("u_param3", UniformType::_4f),
    ]
}

/// Compile an arbitrary GLES fragment source against the engine uniform set.
/// Returns the `GlesError` so the runtime loader can fall back to the built-in.
pub fn compile_source(renderer: &mut GlesRenderer, src: &str) -> Result<GlesPixelProgram, GlesError> {
    renderer.compile_custom_pixel_shader(src, &engine_uniforms())
}

/// The baked-in `spacev3.frag` program; a baked-in shader must always compile,
/// so a failure aborts rather than falling back.
pub fn compile_program(renderer: &mut GlesRenderer) -> GlesPixelProgram {
    match compile_source(renderer, include_str!("../draw.element/shaders/spacev3.frag")) {
        Ok(program) => program,
        Err(err) => abort!("Failed to compile GLES shader {err:?}"),
    }
}

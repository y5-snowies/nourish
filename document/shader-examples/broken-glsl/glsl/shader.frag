#version 450 core
// Intentionally BROKEN GLSL — a compile-error test. The loader logs the naga
// glsl-in parse error and falls back to the built-in parallax.
//
// @prop foo float default=1.0

layout(location = 0) out vec4 frag_color;

void main() {
    frag_color = vec4(1.0, 0.0, 0.0   // <-- missing closing paren + semicolon
}

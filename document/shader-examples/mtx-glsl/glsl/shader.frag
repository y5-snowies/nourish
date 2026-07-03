#version 450 core
// MATRIX CELL: `glsl/` format (desktop GLSL 450) on the VULKAN renderer
// (naga glsl-in → SPIR-V). Feature exercised: round() — a GLSL builtin absent
// from GLSL ES 1.00 — proving modern desktop GLSL compiles for Vulkan.
// @prop steps float default=6.0 min=2.0 max=16.0 step=1.0 label="Posterize steps"
layout(push_constant) uniform Push { vec4 rzt; vec4 pf; vec4 la; vec4 params[2]; } pc;
layout(location = 0) out vec4 frag_color;
void main() {
    vec2 res = pc.rzt.xy;
    float t = pc.rzt.w;
    float steps = pc.params[0].x;
    vec2 uv = gl_FragCoord.xy / res;
    float v = sin(uv.x * 6.0 + t) * 0.5 + 0.5;
    v = round(v * steps) / steps;                 // <- round(): ES-3/desktop only
    frag_color = vec4(vec3(0.20, 0.60, 0.30) * v + vec3(0.03, 0.02, 0.05), 1.0);
}

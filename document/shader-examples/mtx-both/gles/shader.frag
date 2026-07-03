precision highp float;
// MATRIX CELL: BOTH renderers — this file (gles/, GLSL ES 3.00) runs on GLES;
// the sibling vulkan/shader.wgsl runs on Vulkan. Same posterized look.
// @prop steps float default=6.0 min=2.0 max=16.0 step=1.0 label="Posterize steps"
uniform float u_time;
uniform vec2  u_resolution;
uniform float alpha;
uniform vec4  u_param0; // @prop slot 0 = steps
void main() {
    vec2 uv = gl_FragCoord.xy / u_resolution;
    float steps = u_param0.x;
    float v = sin(uv.x * 6.0 + u_time) * 0.5 + 0.5;
    float q = round(v * steps) / steps;           // ES-3 harness required
    gl_FragColor = vec4(vec3(0.5, 0.5, 0.9) * q + vec3(0.03, 0.02, 0.06), 1.0) * alpha;
}

precision highp float;
// MATRIX CELL: `gles/` format on the GLES renderer, compiled as GLSL ES 3.00.
// Feature exercised: round() + a dynamic-bound loop — BOTH absent from GLSL ES
// 1.00. This renders ONLY because the ES-3 harness is active; on the old
// #version 100 path it would fail to compile.
// @prop steps float default=6.0 min=2.0 max=16.0 step=1.0 label="Posterize steps"
uniform float u_time;
uniform vec2  u_resolution;
uniform float alpha;
uniform vec4  u_param0; // @prop slot 0 = steps
void main() {
    vec2 uv = gl_FragCoord.xy / u_resolution;
    float steps = u_param0.x;
    // dynamic-length loop (loop bound from a uniform) — invalid in ES 1.00.
    float acc = 0.0;
    for (int i = 0; i < int(steps); i++) {
        acc += sin(uv.x * 6.0 + float(i) * 0.6 + u_time) * 0.5 + 0.5;
    }
    float v = round(acc / steps * 6.0) / 6.0;     // <- round(): ES-3 only
    gl_FragColor = vec4(vec3(0.75, 0.40, 0.20) * v + vec3(0.05, 0.02, 0.03), 1.0) * alpha;
}

precision highp float;
// Ripple — native GLES-only bundle (ES-1.00). Runs on the GLES backend; on the
// Vulkan backend there is no vulkan/wgsl/glsl folder, so the loader falls back
// to the built-in parallax. Demonstrates per-renderer fallback to built-in.
//
// @prop frequency float default=24.0 min=1.0 max=60.0 step=1.0 label="Ripple frequency" group="Ripple"

uniform float u_time;
uniform vec2  u_resolution;
uniform float alpha;
uniform vec4  u_param0; // @prop slot 0 = frequency

void main() {
    vec2 uv = gl_FragCoord.xy / u_resolution;
    float d = distance(uv, vec2(0.5));
    float r = 0.5 + 0.5 * sin(d * u_param0.x - u_time * 2.0);
    vec3 col = mix(vec3(0.02, 0.03, 0.08), vec3(0.2, 0.4, 0.7), r);
    gl_FragColor = vec4(col, 1.0) * alpha;
}

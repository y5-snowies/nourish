precision highp float;
// Dual-source bundle (GLES half, ES-1.00). Runs raw through smithay's custom
// pixel-shader path on the GLES backend. Declares the engine uniforms it needs;
// writes gl_FragColor. Sibling vulkan/shader.wgsl handles the Vulkan backend.
//
// @prop warp float default=0.20 min=0.0 max=1.0 step=0.01 label="Warp amount" group="Grid"

uniform float u_time;
uniform vec2  u_resolution;
uniform float alpha;
uniform vec4  u_param0; // @prop slot 0 = warp

void main() {
    vec2 res = u_resolution;
    float t = u_time;
    float warp = u_param0.x;
    vec2 uv = (gl_FragCoord.xy / res - 0.5) * vec2(res.x / max(res.y, 1.0), 1.0);
    uv += warp * vec2(sin(uv.y * 4.0 + t), cos(uv.x * 4.0 + t));
    vec2 g = abs(fract(uv * 8.0) - 0.5);
    float line = smoothstep(0.06, 0.0, min(g.x, g.y));
    vec3 col = mix(vec3(0.03, 0.02, 0.06), vec3(0.2, 0.5, 0.9), line);
    gl_FragColor = vec4(col, 1.0) * alpha;
}

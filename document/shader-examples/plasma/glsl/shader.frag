#version 450 core
// Plasma — single-source desktop-450-core GLSL bundle. On Vulkan the loader
// compiles this fragment to SPIR-V via naga (glsl-in) and pairs it with a
// generated fullscreen vertex. (On GLES it falls back: this is desktop GLSL,
// not the ES-1.00 the smithay pixel-shader path runs.)
//
// @prop scale float default=10.0 min=1.0 max=32.0 step=0.5 label="Plasma scale" group="Plasma"

layout(push_constant) uniform Push {
    vec4 res_zoom_time; // xy = resolution, z = zoom, w = time
    vec4 pan_flow;
    vec4 lock_alpha;
    vec4 params[2];     // @prop slot 0 = scale
} pc;

layout(location = 0) out vec4 frag_color;

void main() {
    vec2 res = pc.res_zoom_time.xy;
    float t = pc.res_zoom_time.w;
    float scale = pc.params[0].x; // @prop scale
    vec2 uv = gl_FragCoord.xy / res;
    float v = sin(uv.x * scale + t)
            + sin(uv.y * scale + t * 1.3)
            + sin((uv.x + uv.y) * scale + t * 0.7);
    vec3 col = 0.5 + 0.5 * cos(vec3(0.0, 2.0, 4.0) + v + t * 0.2);
    frag_color = vec4(col * 0.6, 1.0);
}

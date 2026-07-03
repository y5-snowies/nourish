precision highp float;

uniform float u_time;
uniform vec2  u_pan;
uniform vec2  u_flow_offset;
uniform float u_zoom;
uniform vec2  u_resolution;
uniform float alpha;
uniform float u_lock_amount;   // 0 = space, 1 = locked; CPU eases between
uniform vec4  u_param0;         // @prop slots 0..3 (drift/density/nebula/vignette amount)
uniform vec4  u_param1;         // @prop slots 4..7 (vignette radius/softness, ...)

float hash(vec2 p){ return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453); }
float noise(vec2 p){
    vec2 i=floor(p), f=fract(p);
    f=f*f*(3.0-2.0*f);
    return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),
               mix(hash(i+vec2(0,1)),hash(i+vec2(1,1)),f.x), f.y);
}
float fbm(vec2 p){
    float v=0., a=0.5;
    for(int i=0;i<5;i++){ v+=a*noise(p); p*=2.0; a*=0.5; }
    return v;
}

float sdCircle(vec2 p, float r){ return length(p)-r; }

vec3 draw_planet(vec3 col, vec2 uv, vec2 center, float radius,
                 vec3 lightSide, vec3 darkSide, vec2 lightDir, float bandFreq) {
    vec2 pp = uv - center;
    float d = sdCircle(pp, radius);
    float mask = smoothstep(0.004, -0.004, d);
    if (mask <= 0.0) return col;

    float lit = smoothstep(-radius*0.6, radius*0.6, dot(pp, lightDir));
    vec3 base = mix(darkSide, lightSide, lit);

    if (bandFreq > 0.0) {
        float band = sin(pp.y * bandFreq + center.x*3.0) * 0.5 + 0.5;
        float bandNoise = fbm(pp * 15.0) * 0.15;
        base = mix(base, base * 0.75, smoothstep(0.2, 0.8, band + bandNoise));
    }

    float rim = smoothstep(radius * 0.5, radius, length(pp));
    float rimLit = smoothstep(-radius * 0.2, radius, dot(pp, lightDir));
    vec3 atmosphere = lightSide * rim * rimLit * 0.5;

    return mix(col, base + atmosphere, mask);
}

// LOCK: soft drifting fog (the "moon behind clouds" look).
// Horizontally stretched + domain-warped fbm so it reads as layered wisps.
float cloud_fog(vec2 uv, float t){
    vec2 q = uv * vec2(1.6, 2.6);
    q.x += t * 0.025;          // sideways drift
    q.y += t * 0.006;
    vec2 w = vec2(fbm(q*0.6 + t*0.02),
                  fbm(q*0.6 + 5.2 - t*0.015));
    return fbm(q + w*1.5);
}

// LOCK: a single detailed moon — maria, crater mottling, limb darkening.
// LOCK: single moon with limb darkening + a soft diagonal terminator.
vec3 draw_moon(vec3 col, vec2 uv, vec2 center, float radius,
               vec2 lightDir, float brightness){
    vec2 pp = uv - center;
    float r = length(pp);
    float mask = smoothstep(0.003, -0.003, r - radius);
    if (mask <= 0.0) return col;

    vec2 sp = pp / radius;
    float z = sqrt(max(0.0, 1.0 - dot(sp, sp)));

    float maria  = fbm(sp * 1.6 + 7.0);
    float seas   = smoothstep(0.30, 0.62, maria);
    float crater = fbm(sp * 7.0 + 2.0);

    vec3 light = vec3(0.93, 0.92, 0.86);
    vec3 dark  = vec3(0.58, 0.60, 0.66);
    vec3 surf  = mix(light, dark, seas * 0.8);
    surf *= mix(0.9, 1.08, crater);
    surf *= mix(0.65, 1.0, pow(z, 0.45));        // limb darkening

    // diagonal shadow: one half dimmed, soft floor so it's "a bit" shadowed
    float lit = smoothstep(-0.35, 0.55, dot(sp, lightDir));
    surf *= mix(0.22, 1.0, lit);

    return mix(col, surf * brightness, mask);
}
// LOCK: faint distant galaxy — soft elliptical blob with a brighter core
float galaxy(vec2 uv, vec2 c, float rot, vec2 scale){
    vec2 p = uv - c;
    float s = sin(rot), co = cos(rot);
    p = mat2(co, -s, s, co) * p;
    p /= scale;
    float r2 = dot(p, p);
    return exp(-r2 * 6.0) * 0.6 + exp(-r2 * 45.0) * 0.4; // halo + core
}

void main() {
    // @prop-driven knobs (see shader.builtin): drift / density / nebula / vignette.
    float drift_speed = u_param0.x;
    float star_density = u_param0.y;
    float nebula = u_param0.z;
    float vignette = u_param0.w;
    float vigRadius = u_param1.x;
    float vigSoftness = u_param1.y;

    vec2 uv = (gl_FragCoord.xy - 0.5*u_resolution) / u_resolution.y;
    // screenUv is zoom-independent so the vignette frames the display, not the
    // world; the scene uv below is divided by zoom as usual.
    vec2 screenUv = uv;
    uv /= u_zoom;

    vec2 pan = vec2(u_pan.x, -u_pan.y);

    vec3 col = mix(vec3(0.01, 0.015, 0.04), vec3(0.04, 0.02, 0.09), gl_FragCoord.y/u_resolution.y);

    vec2 nebUv = uv*1.5 + pan*0.0002 + u_flow_offset*0.0003 + vec2(u_time*0.01, u_time*0.005)*drift_speed;
    float n = fbm(nebUv);
    float n2 = fbm(nebUv * 2.5 - vec2(u_time * 0.015)*drift_speed);
    col += mix(vec3(0.25, 0.05, 0.35), vec3(0.05, 0.20, 0.45), n) * pow(n, 1.8) * 0.5 * nebula;
    col += vec3(0.1, 0.3, 0.4) * pow(n2, 3.0) * 0.25 * nebula;

    for(int i=1; i<=3; i++){
        float fi = float(i);
        float depth = fi * 0.5;
        vec2 sp = uv * (45.0/depth) + pan * 0.001 * depth;
        vec2 id = floor(sp);
        vec2 fp = fract(sp) - 0.5;
        float h = hash(id);
        if (h > 1.0 - 0.04 * star_density) {
            float twink = 0.5 + 0.5*sin(u_time*1.5 + h*50.0);
            float d = length(fp);
            vec3 starCol = mix(vec3(0.7, 0.9, 1.0), vec3(1.0, 0.85, 0.7), fract(h * 133.7));
            float glow = smoothstep(0.06, 0.0, d) + smoothstep(0.2, 0.0, d) * 0.3;
            col += starCol * glow * twink / depth;
        }
    }

    {
        vec2 drift = -u_flow_offset * 0.0007 + vec2(u_time*0.12*drift_speed, 0.0);
        vec2 p = uv * vec2(1.8, 12.0) + drift;
        vec2 id = floor(p);
        vec2 f  = fract(p) - 0.5;
        float h = hash(id);
        if (h > 0.86) {
            float streak = smoothstep(0.5, 0.0, abs(f.y)*5.0) * smoothstep(0.5, 0.0, abs(f.x)*1.1);
            col += vec3(0.45, 0.65, 1.0) * streak * (h - 0.86) * 3.5;
        }
    }

    col = draw_planet(col, uv,
        vec2(-0.65, 0.30) - pan*0.00015, 0.07,
        vec3(0.85, 0.85, 0.90), vec3(0.18, 0.18, 0.22),
        normalize(vec2(1.0, 0.3)), 0.0);

    col = draw_planet(col, uv,
        vec2(0.70, 0.15) - pan*0.00030, 0.13,
        vec3(0.35, 0.65, 0.55), vec3(0.08, 0.15, 0.12),
        normalize(vec2(-0.6, 0.4)), 0.0);

    col = draw_planet(col, uv,
        vec2(-0.40, -0.30) - pan*0.00055, 0.22,
        vec3(0.90, 0.60, 0.35), vec3(0.15, 0.05, 0.08),
        normalize(vec2(0.7, 0.5)), 15.0);
// ---------------- LOCK SCREEN TRANSITION ----------------
    float L = clamp(u_lock_amount, 0.0, 1.0);
    L = L * L * (3.0 - 2.0 * L);

    if (L > 0.001) {
        float drift = u_time * 0.003;   // barely-there outward creep

        // Deep, cold void — darker & bluer than known space
        vec3 lcol = mix(vec3(0.004, 0.006, 0.018),
                        vec3(0.010, 0.015, 0.040),
                        clamp(uv.y*0.5 + 0.5, 0.0, 1.0));

        // The home galaxy, now just a faint diffuse smear far behind us
        float bandAxis = dot(uv, normalize(vec2(0.6, -0.8))) + 0.55;
        float bandShape = exp(-bandAxis * bandAxis * 4.0);
        float bandTex = fbm(uv * 1.3 + vec2(drift, -2.0));
        lcol += mix(vec3(0.04, 0.05, 0.09), vec3(0.07, 0.06, 0.11), bandTex)
                * bandShape * bandTex * 0.5;

        // Deep field: countless tiny, dim, motionless stars.
        // Layer 2 tinted faintly red — the most distant / redshifted ones.
        for (int i = 1; i <= 2; i++) {
            float dens = (i == 1) ? 55.0 : 95.0;
            float thr  = (i == 1) ? 0.980 : 0.992;
            vec2 sp = uv * dens + pan * 0.0002 * float(i);
            vec2 id = floor(sp);
            vec2 fp = fract(sp) - 0.5;
            float h = hash(id);
            if (h > thr) {
                float d = length(fp);
                float core = smoothstep(0.12, 0.0, d);
                vec3 sc = (i == 2) ? vec3(0.45, 0.30, 0.26)   // far / red
                                   : vec3(0.45, 0.52, 0.68);  // near / cold
                lcol += sc * core * ((i == 2) ? 0.35 : 0.60);
            }
        }

        // A few other galaxies — whole island universes, rendered as specks.
        // This is where the "overwhelming scale" comes from.
        lcol += vec3(0.16, 0.15, 0.21)
              * galaxy(uv, vec2( 0.52,  0.34) + drift, 0.6,  vec2(0.13, 0.045)) * 0.55;
        lcol += vec3(0.13, 0.13, 0.19)
              * galaxy(uv, vec2(-0.58, -0.22) + drift, -0.3, vec2(0.09, 0.030)) * 0.45;
        lcol += vec3(0.12, 0.11, 0.17)
              * galaxy(uv, vec2( 0.05, -0.40) + drift, 1.2,  vec2(0.05, 0.020)) * 0.40;

        // Vignette: deepen the void toward the edges, frame the lock UI
        float vig = smoothstep(1.25, 0.15, length(uv));
        lcol *= mix(0.30, 1.0, vig);

        col = mix(col, lcol, L);
    }

    // Optional vignette (slots 3..5): screen-space (zoom-independent) with
    // knob-driven radius / softness so the framing stays consistent across zoom.
    float vig = smoothstep(vigRadius, vigRadius - vigSoftness, length(screenUv));
    col *= mix(1.0, vig, clamp(vignette, 0.0, 1.0));
    gl_FragColor = vec4(col, 1.0) * alpha * 0.75;
}

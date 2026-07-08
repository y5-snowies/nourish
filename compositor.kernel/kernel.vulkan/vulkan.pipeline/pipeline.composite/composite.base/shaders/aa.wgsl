// world anti-aliasing composite for the textured (window + iced) arm. Separate
// image+sampler bindings (naga cannot emit combined image-samplers, so this
// cannot reuse the GLSL composite's combined `sampler2D`). ONE pipeline serves
// every AA mode: `params2.x` selects the path — 0 = the classic minification
// arm (bilinear / anisotropic / trilinear sampler + optional N×N supersample +
// unsharp), 1 = FSR EASU (edge-adaptive upscale), 2 = FSR RCAS (contrast-
// adaptive sharpen). All are chosen per draw on the CPU, so switching methods
// never rebuilds the pipeline.
//
// Only the textured `DrawOp` arm is routed here; solids and the parallax
// shader-pass (background) never touch it.
//
// EASU/RCAS are faithful ports of AMD FidelityFX FSR 1.0 (`ffx_fsr1.h`,
// `FsrEasuF`/`FsrRcasF`), reformulated to fetch integer texels via `textureLoad`
// instead of the reference gather/const-packing. RGB is filtered by the FSR
// kernel; alpha is taken from a straight bilinear sample (premultiplied, so the
// two filter independently). These are magnification filters: they reconstruct
// edges when a surface is drawn larger than its buffer.

struct Push {
    dst: vec4<f32>,    // x, y, w, h in NDC
    src: vec4<f32>,    // u, v, w, h in UV
    color: vec4<f32>,  // (1,1,1,alpha) for textured
    // x = taps per axis (>=1); y = footprint spread; z = sharpen amount; w = LOD bias
    params: vec4<f32>,
    // x = mode (0 classic, 1 EASU, 2 RCAS); y = RCAS strength (0..1);
    // z,w = source texture width,height in texels (for EASU/RCAS texel fetch)
    params2: vec4<f32>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let p = pc.dst.xy + corner * pc.dst.zw;
    out.uv = pc.src.xy + corner * pc.src.zw;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    return out;
}

// --- shared helpers -------------------------------------------------------

// Fetch a source texel by integer coordinate, edge-clamped to the image.
fn load_texel(p: vec2<i32>, dim: vec2<i32>) -> vec3<f32> {
    let q = clamp(p, vec2<i32>(0, 0), dim - vec2<i32>(1, 1));
    return textureLoad(src_tex, q, 0).rgb;
}

// FSR luma weighting: B*0.5 + (R*0.5 + G).
fn fsr_luma(c: vec3<f32>) -> f32 {
    return c.b * 0.5 + (c.r * 0.5 + c.g);
}

// --- FSR EASU (edge-adaptive spatial upsampling) --------------------------

// FsrEasuSetF: one 2x2 edge estimate, returned as (dir.x, dir.y, len) weighted
// by `w` for accumulation across the four quadrants.
fn easu_set(w: f32, lA: f32, lB: f32, lC: f32, lD: f32, lE: f32) -> vec3<f32> {
    let dc = lD - lC;
    let cb = lC - lB;
    let dirX = lD - lB;
    var lenX = 1.0 / max(max(abs(dc), abs(cb)), 1e-6);
    lenX = clamp(abs(dirX) * lenX, 0.0, 1.0);
    lenX = lenX * lenX;
    let ec = lE - lC;
    let ca = lC - lA;
    let dirY = lE - lA;
    var lenY = 1.0 / max(max(abs(ec), abs(ca)), 1e-6);
    lenY = clamp(abs(dirY) * lenY, 0.0, 1.0);
    lenY = lenY * lenY;
    return vec3<f32>(dirX * w, dirY * w, (lenX + lenY) * w);
}

// FsrEasuTapF: one directional Lanczos-ish tap, returned as (color*w, w).
fn easu_tap(off: vec2<f32>, dir: vec2<f32>, len2: vec2<f32>, lob: f32, clp: f32, c: vec3<f32>) -> vec4<f32> {
    var v = vec2<f32>(
        off.x * dir.x + off.y * dir.y,
        off.x * (-dir.y) + off.y * dir.x,
    );
    v = v * len2;
    var d2 = v.x * v.x + v.y * v.y;
    d2 = min(d2, clp);
    var wB = (2.0 / 5.0) * d2 - 1.0;
    var wA = lob * d2 - 1.0;
    wB = wB * wB;
    wA = wA * wA;
    wB = (25.0 / 16.0) * wB - (25.0 / 16.0 - 1.0);
    let w = wB * wA;
    return vec4<f32>(c * w, w);
}

fn easu(uv: vec2<f32>, tex: vec2<f32>) -> vec3<f32> {
    let dim = vec2<i32>(i32(tex.x), i32(tex.y));
    // Continuous input-texel position; tap f (0,0) is the top-left of the
    // central 2x2, `pp` the fractional resolve position within it.
    let sp = uv * tex - vec2<f32>(0.5, 0.5);
    let b0 = floor(sp);
    let pp = sp - b0;
    let ip = vec2<i32>(i32(b0.x), i32(b0.y));

    //      b c
    //    e f g h
    //    i j k l
    //      n o
    let cb = load_texel(ip + vec2<i32>( 0, -1), dim);
    let cc = load_texel(ip + vec2<i32>( 1, -1), dim);
    let ce = load_texel(ip + vec2<i32>(-1,  0), dim);
    let cf = load_texel(ip + vec2<i32>( 0,  0), dim);
    let cg = load_texel(ip + vec2<i32>( 1,  0), dim);
    let ch = load_texel(ip + vec2<i32>( 2,  0), dim);
    let ci = load_texel(ip + vec2<i32>(-1,  1), dim);
    let cj = load_texel(ip + vec2<i32>( 0,  1), dim);
    let ck = load_texel(ip + vec2<i32>( 1,  1), dim);
    let cl = load_texel(ip + vec2<i32>( 2,  1), dim);
    let cn = load_texel(ip + vec2<i32>( 0,  2), dim);
    let co = load_texel(ip + vec2<i32>( 1,  2), dim);

    let bL = fsr_luma(cb); let cL = fsr_luma(cc);
    let eL = fsr_luma(ce); let fL = fsr_luma(cf); let gL = fsr_luma(cg); let hL = fsr_luma(ch);
    let iL = fsr_luma(ci); let jL = fsr_luma(cj); let kL = fsr_luma(ck); let lL = fsr_luma(cl);
    let nL = fsr_luma(cn); let oL = fsr_luma(co);

    var acc = easu_set((1.0 - pp.x) * (1.0 - pp.y), bL, eL, fL, gL, jL);
    acc = acc + easu_set(       pp.x  * (1.0 - pp.y), cL, fL, gL, hL, kL);
    acc = acc + easu_set((1.0 - pp.x) *        pp.y , fL, iL, jL, kL, nL);
    acc = acc + easu_set(       pp.x  *        pp.y , gL, jL, kL, lL, oL);
    var dir = acc.xy;
    var len = acc.z;

    // Normalize direction; derive the anisotropic filter shape.
    let dir2 = dir * dir;
    var dirR = dir2.x + dir2.y;
    let zro = dirR < (1.0 / 32768.0);
    dirR = inverseSqrt(max(dirR, 1e-6));
    if (zro) { dirR = 1.0; dir.x = 1.0; }
    dir = dir * vec2<f32>(dirR, dirR);
    len = len * 0.5;
    len = len * len;
    let stretch = (dir.x * dir.x + dir.y * dir.y) / max(max(abs(dir.x), abs(dir.y)), 1e-6);
    let len2 = vec2<f32>(1.0 + (stretch - 1.0) * len, 1.0 + (-0.5) * len);
    let lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
    let clp = 1.0 / lob;

    // Ringing clamp: keep the result within the central 2x2 min/max.
    let mn4 = min(min(cf, cg), min(cj, ck));
    let mx4 = max(max(cf, cg), max(cj, ck));

    var a = easu_tap(vec2<f32>( 0.0, -1.0) - pp, dir, len2, lob, clp, cb);
    a = a + easu_tap(vec2<f32>( 1.0, -1.0) - pp, dir, len2, lob, clp, cc);
    a = a + easu_tap(vec2<f32>(-1.0,  1.0) - pp, dir, len2, lob, clp, ci);
    a = a + easu_tap(vec2<f32>( 0.0,  1.0) - pp, dir, len2, lob, clp, cj);
    a = a + easu_tap(vec2<f32>( 0.0,  0.0) - pp, dir, len2, lob, clp, cf);
    a = a + easu_tap(vec2<f32>(-1.0,  0.0) - pp, dir, len2, lob, clp, ce);
    a = a + easu_tap(vec2<f32>( 1.0,  1.0) - pp, dir, len2, lob, clp, ck);
    a = a + easu_tap(vec2<f32>( 2.0,  1.0) - pp, dir, len2, lob, clp, cl);
    a = a + easu_tap(vec2<f32>( 2.0,  0.0) - pp, dir, len2, lob, clp, ch);
    a = a + easu_tap(vec2<f32>( 1.0,  0.0) - pp, dir, len2, lob, clp, cg);
    a = a + easu_tap(vec2<f32>( 1.0,  2.0) - pp, dir, len2, lob, clp, co);
    a = a + easu_tap(vec2<f32>( 0.0,  2.0) - pp, dir, len2, lob, clp, cn);

    let outc = a.rgb / max(a.w, 1e-6);
    return min(mx4, max(mn4, outc));
}

// --- FSR RCAS (robust contrast-adaptive sharpening) -----------------------

const FSR_RCAS_LIMIT: f32 = 0.25 - (1.0 / 16.0);

// RCAS core over five pre-sampled colors: center `e` and its 4-neighborhood
// (b above, d left, f right, h below). Taking colors (not texel loads) lets the
// caller feed EITHER raw source taps OR EASU-reconstructed neighbors — the
// latter being the canonical FSR1 EASU->RCAS chain.
fn rcas_combine(b: vec3<f32>, d: vec3<f32>, e: vec3<f32>, f: vec3<f32>, h: vec3<f32>, sharp: f32) -> vec3<f32> {
    let mn4 = min(min(b, d), min(f, h));
    let mx4 = max(max(b, d), max(f, h));

    // Per-channel lobe (how far the center may be pushed without clipping the
    // 4-neighborhood). peakC = (1.0, -4.0) in the reference.
    let hitMin = min(mn4, e) / max(4.0 * mx4, vec3<f32>(1e-6));
    let hitMax = (vec3<f32>(1.0) - max(mx4, e)) / min(4.0 * mn4 - 4.0, vec3<f32>(-1e-6));
    let lobeRGB = max(-hitMin, hitMax);
    let lobeMax = max(max(lobeRGB.r, lobeRGB.g), lobeRGB.b);
    let lobe = max(-FSR_RCAS_LIMIT, min(lobeMax, 0.0)) * sharp;

    let rcpL = 1.0 / (4.0 * lobe + 1.0);
    return (b * lobe + d * lobe + f * lobe + h * lobe + e) * rcpL;
}

// Base reconstruction the FSR path sharpens over: EASU when enabled, else a
// plain bilinear tap. Derivative-free (textureLoad / explicit-LOD sample), so
// it is safe to call under the uniform push-constant branches and at the
// output-pixel neighbor offsets RCAS needs.
fn reconstruct(uv: vec2<f32>, tex: vec2<f32>, easu_on: bool) -> vec3<f32> {
    if (easu_on) {
        return easu(uv, tex);
    }
    return textureSampleLevel(src_tex, src_samp, uv, 0.0).rgb;
}

// --- entry ----------------------------------------------------------------

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // FSR toggles travel in params2 (independent of the classic AA method).
    let easu_on = pc.params2.x > 0.5;
    let rcas_strength = pc.params2.y;
    let rcas_on = rcas_strength > 0.0;
    let tex = pc.params2.zw;

    // FSR path — active when either FSR filter is on. It supplies the base color
    // (EASU reconstruction or plain source) and, if RCAS is on, sharpens it; the
    // classic AA method's sampling is bypassed for this fragment. All branch
    // conditions are push constants (uniform), so derivatives stay well-defined.
    if (easu_on || rcas_on) {
        let alpha = textureSampleLevel(src_tex, src_samp, in.uv, 0.0).a;
        var rgb: vec3<f32>;
        if (rcas_on) {
            // Sharpen over the reconstructed image at output-pixel neighbors.
            let dvx = dpdx(in.uv);
            let dvy = dpdy(in.uv);
            let e = reconstruct(in.uv, tex, easu_on);
            let b = reconstruct(in.uv - dvy, tex, easu_on);
            let d = reconstruct(in.uv - dvx, tex, easu_on);
            let f = reconstruct(in.uv + dvx, tex, easu_on);
            let h = reconstruct(in.uv + dvy, tex, easu_on);
            rgb = rcas_combine(b, d, e, f, h, rcas_strength);
        } else {
            rgb = easu(in.uv, tex);
        }
        return vec4<f32>(rgb, alpha) * pc.color;
    }

    // Classic minification arm (the AA method: SSAA / trilinear / aniso).
    let taps = max(i32(pc.params.x), 1);
    let sharpen = pc.params.z;
    let lod_bias = pc.params.w;
    // No supersample and no sharpen: let the sampler do the work directly
    // (bilinear / aniso / trilinear-mip). `lod_bias` shifts the mip level
    // (negative -> sharper/higher-res) for trilinear.
    if (taps <= 1 && sharpen <= 0.0) {
        return textureSampleBias(src_tex, src_samp, in.uv, lod_bias) * pc.color;
    }

    let px = dpdx(in.uv); // UV span of one output pixel, x
    let py = dpdy(in.uv); // UV span of one output pixel, y

    // Low-pass: N×N box supersample across the pixel footprint (× spread). This
    // removes minification aliasing/shimmer. `spread` 1.0 == a true one-pixel
    // box (crisp); >1 over-blurs.
    var col: vec4<f32>;
    if (taps > 1) {
        let spread = max(pc.params.y, 1.0);
        let dx = px * spread;
        let dy = py * spread;
        var acc = vec4<f32>(0.0);
        let inv = 1.0 / f32(taps);
        for (var j = 0; j < taps; j = j + 1) {
            for (var i = 0; i < taps; i = i + 1) {
                let fx = (f32(i) + 0.5) * inv - 0.5;
                let fy = (f32(j) + 0.5) * inv - 0.5;
                let off = fx * dx + fy * dy;
                acc = acc + textureSampleLevel(src_tex, src_samp, in.uv + off, 0.0);
            }
        }
        col = acc / f32(taps * taps);
    } else {
        col = textureSampleBias(src_tex, src_samp, in.uv, lod_bias);
    }

    // Sharpen: unsharp mask on the ALREADY-anti-aliased result. `wide` is a
    // cheap 4-tap neighbourhood average at ~2px; boosting (col - wide) raises
    // edge contrast (crisper text/borders) WITHOUT reintroducing source
    // aliasing, since it operates on the filtered image, not the raw texels.
    if (sharpen > 0.0) {
        let r = 2.0;
        let wide = 0.25 * (
            textureSampleLevel(src_tex, src_samp, in.uv + px * r, 0.0)
            + textureSampleLevel(src_tex, src_samp, in.uv - px * r, 0.0)
            + textureSampleLevel(src_tex, src_samp, in.uv + py * r, 0.0)
            + textureSampleLevel(src_tex, src_samp, in.uv - py * r, 0.0)
        );
        col = clamp(col + sharpen * (col - wide), vec4<f32>(0.0), vec4<f32>(1.0));
    }

    return col * pc.color;
}

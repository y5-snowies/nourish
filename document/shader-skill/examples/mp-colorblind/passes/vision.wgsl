// "mp-colorblind" — SIMULATE OR CORRECT FOR COLOUR VISION DEFICIENCY.
//
// A demonstration, and a genuinely useful one: run `Simulate` to see what a UI
// looks like to someone with the deficiency, or `Correct` to shift the colours
// that collapse into each other back apart so they can be told apart again.
//
// `type` and `mode` are both `choices=` props, which is the whole point of that
// declaration existing: these are branches the shader takes, and offered as a
// slider ("2.00") the control would be unreadable.
//
// THE MATHS. Simulation is the Brettel/Viénot/Mollon LMS projection: convert to
// the cone response space, collapse the missing cone onto the plane the remaining
// two span, convert back. Correction ("daltonisation") then takes the error the
// simulation introduced — what that eye cannot see — and redistributes it onto
// the channels it can, which is why the corrected image looks wrong to normal
// vision and right to the eye it is for.
//
// @prop type      int   default=1 choices="Protan,Deutan,Tritan"  label="Deficiency" group="Vision"
// @prop mode      int   default=0 choices="Simulate,Correct"      label="Mode"       group="Vision"
// @prop amount    float default=1.0 min=0.0 max=1.0 step=0.01     label="Amount"     group="Vision"

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var scene: texture_2d<f32>;

// Linear sRGB → LMS cone response (Hunt-Pointer-Estevez, D65).
fn to_lms(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(c, vec3<f32>(0.31399022, 0.63951294, 0.04649755)),
        dot(c, vec3<f32>(0.15537241, 0.75789446, 0.08670142)),
        dot(c, vec3<f32>(0.01775239, 0.10944209, 0.87256922)),
    );
}

fn to_rgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(c, vec3<f32>( 5.47221206, -4.64196010,  0.16963708)),
        dot(c, vec3<f32>(-1.12524190,  2.29317094, -0.16789520)),
        dot(c, vec3<f32>( 0.02980165, -0.19318073,  1.16364789)),
    );
}

/// Collapse the missing cone. Each row replaces that cone's response with the
/// best linear estimate from the other two.
fn simulate(lms: vec3<f32>, kind: i32) -> vec3<f32> {
    if (kind == 0) {          // Protan — no long-wave (red) cone
        return vec3<f32>(
            0.0 * lms.x + 1.05118294 * lms.y + -0.05116099 * lms.z,
            lms.y,
            lms.z,
        );
    } else if (kind == 1) {   // Deutan — no medium-wave (green) cone
        return vec3<f32>(
            lms.x,
            0.9513092 * lms.x + 0.0 * lms.y + 0.04866992 * lms.z,
            lms.z,
        );
    }
    // Tritan — no short-wave (blue) cone
    return vec3<f32>(
        lms.x,
        lms.y,
        -0.86744736 * lms.x + 1.86727089 * lms.y + 0.0 * lms.z,
    );
}

// The picture arrives gamma-encoded; the cone transform is only meaningful on
// LINEAR light, so decode, work, re-encode. Skipping this is the usual reason a
// hand-rolled colour-blind filter looks washed out.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}
fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let src = textureSampleLevel(scene, samp, frag.xy / res, 0.0).rgb;
    let kind = i32(round(pc.params[0].x));
    let mode = i32(round(pc.params[0].y));
    let amount = clamp(pc.params[0].z, 0.0, 1.0);

    let lin = srgb_to_linear(src);
    let seen = to_rgb(simulate(to_lms(lin), kind));

    var out: vec3<f32>;
    if (mode == 1) {
        // CORRECT — the error is what this eye cannot see. Push it onto the
        // channels it can: red's loss into green and blue, and so on.
        let err = lin - seen;
        var shift: vec3<f32>;
        if (kind == 2) {
            shift = vec3<f32>(err.b * 0.7, err.b * 0.7, 0.0);
        } else {
            shift = vec3<f32>(0.0, err.r * 0.7 + err.g * 0.7, err.r * 0.7 + err.b * 0.7);
        }
        out = clamp(lin + shift, vec3<f32>(0.0), vec3<f32>(1.0));
    } else {
        out = clamp(seen, vec3<f32>(0.0), vec3<f32>(1.0));
    }

    var col = linear_to_srgb(mix(lin, out, amount));
    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

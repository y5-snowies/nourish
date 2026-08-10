// TB — `textures` as a MATERIAL: a tiling surface, lit by the pointer.
//
// The third thing a texture is for. `tb-texture-sheet` samples art and
// `tb-texture-grade` samples a table; this samples a measured surface. `paper.png`
// stores a height field in R and its two slope components in G and B, which is
// enough to light the sheet as though it had relief — the grain catches the light
// and shades away from it, and none of it is in the geometry.
//
// WHAT TO LOOK FOR
// ----------------
// Move the pointer. The whole surface re-lights around it: fibres on the near side
// of the light go bright, the far side falls into shadow, and the effect follows
// the cursor continuously rather than switching between states. Hold the left
// button and the light brightens and tightens.
//
// Turn `depth` to 0 for flat paper and back up — that is the slope channels doing
// all of the work. Turn `tiles` up and the same 256x256 image covers the screen
// many times with no visible seam.
//
// SEAMLESS TILING, WITHOUT A REPEAT SAMPLER
// -----------------------------------------
// There is ONE sampler and it clamps at the edge. That is right for an atlas and
// wrong here, so the wrap is done in the shader: `fract()` on the scaled UV. The
// image is authored to be seamless (its noise is a sum of waves at whole numbers
// of cycles per tile, so the tile matches itself at every edge — see
// `make-texture-assets.py`), which is what makes that legal.
//
// The one thing to know: `fract` produces a discontinuity at each tile boundary,
// and the hardware picks a mip level from screen-space derivatives, so an
// automatic-LOD sample would band along every seam. `textureSampleLevel(..., 0.0)`
// sidesteps it by never asking for a derivative — which is also why every shipped
// bundle uses it.
//
// `"srgb": false` again, and for the same reason as the LUT: a height and two
// slopes are numbers, not colours. Linearising them would bend the relief.
//
// @prop tiles  float default=6.0  min=1.0  max=24.0 step=1.0  label="Tiles across"  group="Paper"
// @prop depth  float default=1.00 min=0.0  max=3.0  step=0.05 label="Grain depth"   group="Paper"
// @prop reach  float default=0.55 min=0.10 max=2.00 step=0.01 label="Light falloff" group="Light"
// @prop warm   float default=0.60 min=0.0  max=1.0  step=0.01 label="Light warmth"  group="Light"
// @prop base   float default=0.42 min=0.0  max=1.0  step=0.01 label="Paper tone"    group="Paper"

// COORDINATE CHOICE — SCREEN-ANCHORED, DELIBERATELY
// -------------------------------------------------
// `tiles` is a SCREEN length: the grain stays the same size on the display at
// every zoom level, and zooming the world out does not make the paper finer.
// That is correct here, because this sheet is over the display rather than in the
// world — the same call a vignette, scanlines or film grain make.
//
// It is the WRONG call for anything that belongs to the world or to a window. The
// engine transforms the positions it hands you (`windows.rects`, `pointer.at`)
// but not the lengths you write yourself, so a length left in screen units grows
// relative to everything else as the user zooms out. If this material were meant
// to be a surface the desktop sits ON, `tiles` would be multiplied by
// `pc.res_zoom_time.z`. See SKILL.md 3.1, and `mp-parallax`'s `lib/parallax.wgsl`
// for the world-anchored convention.

struct Push {
    res_zoom_time: vec4<f32>,
    pan_flow: vec4<f32>,
    lock_alpha: vec4<f32>,
    params: array<vec4<f32>, 2>,
};
var<immediate> pc: Push;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var paper: texture_2d<f32>;

struct Pointer {
    // xy = screen UV, z = held buttons, w = reserved
    at: vec4<f32>,
    // x = last pressed, y = last released, z / w = reserved
    moment: vec4<f32>,
};
@group(1) @binding(3) var<uniform> pointer: Pointer;

const BTN_LEFT: u32 = 1u;

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let res = max(pc.res_zoom_time.xy, vec2<f32>(1.0));
    let uv = frag.xy / res;
    let aspect = res.x / res.y;

    let tiles = pc.params[0].x;
    let depth = pc.params[0].y;
    let reach = pc.params[0].z;
    let warm = pc.params[0].w;
    let base = pc.params[1].x;

    // Tile in a SQUARE space, so the grain is not stretched on a wide screen —
    // the y axis sets the rate and x follows the aspect.
    let tiled = fract(vec2<f32>(uv.x * aspect, uv.y) * tiles);
    // Explicit level 0: `fract` is discontinuous at every tile edge, and an
    // automatic-LOD sample would read that jump as a huge derivative and band.
    let m = textureSampleLevel(paper, samp, tiled, 0.0);

    // R is height; G and B are the two slopes, signed and biased to 0.5.
    let height = m.r;
    let slope = (m.gb - vec2<f32>(0.5)) * 2.0 * depth;
    // A surface normal from the two slopes. Not normalised in x/y separately —
    // the length is what carries how steep the fibre is.
    let normal = normalize(vec3<f32>(-slope.x, -slope.y, 1.0));

    // The light sits where the hand is. `pointer.at` is screen UV already, so
    // there is no extent to reconcile.
    let held = (u32(pointer.at.z) & BTN_LEFT) != 0u;
    var power = 1.0;
    var falloff = reach;
    if (held) {
        power = 1.6;
        falloff = reach * 0.6;
    }
    let to_light = vec2<f32>((pointer.at.x - uv.x) * aspect, pointer.at.y - uv.y);
    let dist = length(to_light);
    // A light a little above the sheet, so grazing angles at the edges pick out
    // the grain and the centre does not blow out.
    let l = normalize(vec3<f32>(to_light, falloff * 0.75));
    let lambert = max(dot(normal, l), 0.0);
    let fall = power / (1.0 + pow(dist / falloff, 2.0));

    // Paper: a warm neutral, darkened in the troughs of the height field.
    let tone = mix(vec3<f32>(0.86, 0.84, 0.78), vec3<f32>(0.55, 0.52, 0.47), base);
    let shade = tone * (0.55 + 0.45 * height);

    let light_col = mix(vec3<f32>(0.80, 0.86, 1.00), vec3<f32>(1.00, 0.86, 0.66), warm);
    var col = shade * (0.10 + lambert * fall) * light_col;
    // A little of the surface's own colour survives where no light reaches, so
    // the far corners are dim rather than black.
    col = col + shade * 0.045;

    if (pc.lock_alpha.z > 0.5) {
        col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(col, 1.0);
}

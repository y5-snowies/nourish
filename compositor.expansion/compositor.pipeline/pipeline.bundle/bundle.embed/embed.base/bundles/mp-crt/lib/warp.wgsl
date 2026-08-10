// The tube's geometry, in ONE place — the pass samples through it and the engine
// interprets it on the CPU to correct the pointer, so the curve you see and the
// curve the cursor is corrected by cannot be two different functions.
//
// ASPECT-NORMALISED, which is the whole difference from `crt-input-map`'s version
// and the reason this one is usable on a 49" ultrawide.
//
// The naive form corrects by `res.x/res.y` and measures `r2` in those units, so
// the corner radius grows with the aspect: at 16:9 the corner sits at r²≈0.99, at
// 32:9 at r²≈3.2. The same `curve` therefore bends a 32:9 display more than three
// times as hard, and the sides fold in so far the picture leaves the screen.
//
// Dividing r² by the corner's own radius makes it 1.0 at the corner on ANY
// display, so `curve` means "how far the corner moves" everywhere from 16:9 to
// 32:9. The curve is still radial and still aspect-correct — circles stay circles
// — it is only the SCALE that stops depending on the panel.
//
// IT OVERSCANS, so the picture fills the screen and there is no black surround.
//
// A barrel warp pushes samples OUTWARD, so the screen corner asks for a source
// point past the edge of the picture and there is nothing there — which is where
// the black cutoff came from. Real tubes solve this the same way a broadcast
// signal did: draw the picture slightly larger than the visible area, so the
// bezel eats the overscan instead of the viewer seeing the edge of the raster.
//
// Dividing by `1 + k` is exactly that. At the corner `r2` is 1, so the factor
// becomes `(1+k)/(1+k)` = 1 and the screen corner lands precisely on the source
// corner; everywhere inside, the picture is scaled up slightly. Nothing samples
// outside the source at any curvature, so the surround cannot appear.
//
// PURE, BY CONTRACT: no bindings, no textures, no globals, everything by argument.
// That is what makes it griddable, and `shader.hit` enforces it at load. The
// overscan is inside the function rather than applied by the pass, so the pointer
// correction gets it too — a cursor corrected by an un-overscanned curve would
// drift further from the hand the closer it got to an edge.
#define_import_path mp::crt

fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    let k = params.x;
    let a = vec2<f32>(res.x / max(res.y, 1.0), 1.0);
    let c = (uv - vec2<f32>(0.5, 0.5)) * a;
    // The corner's radius in the same aspect-corrected space. `r2` is then 0 at
    // the centre and 1 at the corner, whatever the panel.
    let corner = 0.5 * length(a);
    let r2 = dot(c, c) / max(corner * corner, 0.0001);
    // `1 + k` is the corner's own displacement, so dividing by it normalises the
    // corner back onto the source corner: overscan, exactly enough and no more.
    return vec2<f32>(0.5, 0.5) + c * ((1.0 + k * r2) / (1.0 + max(k, 0.0))) / a;
}

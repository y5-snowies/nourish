// storm-forest — the shared weather field: noise, the flash envelope, rain.
//
// Imported by passes/forest.wgsl (which needs the flash to LIGHT the trees) and
// passes/band.wgsl (which needs the same flash to light the rain, the bolt and
// the windows). One definition, so the whole picture flashes on the same frame.
#define_import_path storm::field

fn s_hash11(x: f32) -> f32 {
    return fract(sin(x * 91.3458) * 47453.5453);
}

fn s_hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

/// Value noise, smoothstep-interpolated.
fn s_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = s_hash21(i);
    let b = s_hash21(i + vec2<f32>(1.0, 0.0));
    let c = s_hash21(i + vec2<f32>(0.0, 1.0));
    let d = s_hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn s_fbm(p: vec2<f32>, octaves: i32) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < octaves; i = i + 1) {
        v = v + amp * s_noise(q);
        q = q * 2.03;
        amp = amp * 0.5;
    }
    return v;
}

/// Ridged noise — the spiky profile that reads as conifers rather than hills.
fn s_ridge(p: vec2<f32>, octaves: i32) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < octaves; i = i + 1) {
        v = v + amp * (1.0 - abs(s_noise(q) * 2.0 - 1.0));
        q = q * 2.11;
        amp = amp * 0.5;
    }
    return v;
}

/// One sub-flash: instant attack at `start`, quadratic decay over `len`.
fn s_pulse(age: f32, start: f32, len: f32) -> f32 {
    let x = (age - start) / max(len, 0.001);
    if (x < 0.0 || x > 1.0) {
        return 0.0;
    }
    let d = 1.0 - x;
    return d * d;
}

/// The brightness of the sky, `age` seconds after a strike.
///
/// Three overlapping sub-flashes — a hard leader, a re-strike, then a long
/// rolling afterglow. Real lightning almost never fires once, and a single
/// clean pulse is the thing that makes a storm shader look like a light switch.
fn flash_curve(age: f32) -> f32 {
    if (age < 0.0) {
        return 0.0;
    }
    let a = s_pulse(age, 0.00, 0.11);
    let b = s_pulse(age, 0.09, 0.08) * 0.8;
    let c = s_pulse(age, 0.24, 0.30) * 0.45;
    let glow = s_pulse(age, 0.05, 1.60) * 0.14;
    return a + b + c + glow;
}

/// One sheet of falling rain. Returns 0..1 streak coverage.
///
/// EVERY UNIT HERE IS IN SCREEN TERMS, on purpose. An earlier version took the
/// fall speed in cell-heights, so the column count silently divided it and the
/// sheet crawled down the screen over about thirteen seconds — long, bright,
/// slow lines, which is a meteor shower and not rain. Rain crosses the frame in
/// well under a second, and reads as many faint fast threads rather than a few
/// bold ones.
///
///   cols   columns across the screen
///   fall   SCREEN-HEIGHTS PER SECOND — the number that makes or breaks it
///   len_uv streak length in screen heights (its motion blur)
///   thick  streak width as a fraction of one column
fn rain_layer(
    uv: vec2<f32>,
    t: f32,
    cols: f32,
    fall: f32,
    len_uv: f32,
    thick: f32,
    slant: f32,
    seed: f32,
) -> f32 {
    var p = uv;
    p.x = p.x + p.y * slant;

    // Rows are derived from the streak length so a cell always holds one streak
    // plus a gap — length and speed then stay independent of each other.
    let rows = max(1.0 / max(len_uv * 4.0, 0.0008), 2.0);

    let gx = p.x * cols;
    let cx = floor(gx);
    let fx = fract(gx);

    let r1 = s_hash21(vec2<f32>(cx, seed));
    // Each column falls at its own rate, so the sheet never marches in step.
    let sp = fall * (0.80 + 0.40 * r1);
    // uv.y is down-positive, so SUBTRACTING t moves the sheet downward.
    let ph = (p.y - t * sp) * rows + r1 * 17.0;
    let y = fract(ph);
    let row = floor(ph);

    // Most cells carry a drop — a sparse sheet reads as debris, not weather.
    let r2 = s_hash21(vec2<f32>(cx * 1.7, row + seed * 3.0));
    if (r2 < 0.30) {
        return 0.0;
    }

    // Head at the bottom of the cell, tail fading up behind it.
    let len_cells = clamp(len_uv * rows, 0.03, 0.85);
    let streak = smoothstep(1.0 - len_cells, 1.0, y);

    // Placed at a hashed offset inside the column, so there is no visible grid.
    let xoff = 0.12 + 0.76 * fract(r2 * 7.31);
    let across = 1.0 - smoothstep(0.0, max(thick, 0.0005), abs(fx - xoff));

    // Depth within the sheet: most threads are faint, a few are near and bright.
    return streak * across * (0.22 + 0.78 * r2 * r2);
}

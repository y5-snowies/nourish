// storm-forest — the shake.
//
// There is NO `hit` block in the manifest: the pointer is deliberately not
// warped to follow the rattle. The correction has to run on the CPU for every
// drawable on every pointer event, and for a displacement that lasts under a
// second it cost more in cursor latency than it bought in accuracy. Clicks land
// on where the window really is while it shakes, which is also the more
// predictable behaviour.
//
// If it is ever wanted back, restore the hit_drawable() entry (git history, or
// examples/crt-rotating-input-both) and add the `hit` block pointing here — the
// functions below are already written to the CPU evaluator's restrictions:
// plain arithmetic only, no textures, derivatives, atomics or globals, and a
// cubic envelope instead of exp().
#define_import_path storm::quake

/// Sin-free hash. This runs inside the per-pointer-event CPU interpreter, where
/// a transcendental is worth many multiplies, and strike_at calls it on every
/// evaluation whether or not anything is shaking.
fn q_hash(n: f32) -> f32 {
    var p = fract(n * 0.1031);
    p = p * (p + 33.33);
    p = p * (p + p);
    return fract(p);
}

/// The most recent lightning strike at time `t`.
///   .x = the moment it happened (absolute, same clock as res_zoom_time.w)
///   .y = its index, which seeds everything else about it
///
/// Strikes sit on a `period` grid jittered by a hash of the slot, so they are
/// irregular but a pure function of the clock — which is what lets the CPU
/// evaluator agree with the shader without being told anything.
fn strike_at(t: f32, period: f32) -> vec2<f32> {
    let p = max(period, 1.0);
    let k = floor(t / p);
    var best_t = -1000.0;
    var best_k = -1000.0;
    // The current slot may not have fired yet, so the previous one is still it.
    for (var j = 0; j < 2; j = j + 1) {
        let kk = k - f32(j);
        let st = kk * p + q_hash(kk + 3.0) * p * 0.7;
        if (st <= t && st > best_t) {
            best_t = st;
            best_k = kk;
        }
    }
    return vec2<f32>(best_t, best_k);
}

/// Cubic decay, 1 at age 0 → 0 at `life`. Cheap, and CPU-evaluator safe.
fn quake_env(age: f32, life: f32) -> f32 {
    if (age < 0.0 || age > life) {
        return 0.0;
    }
    let d = 1.0 - age / life;
    return d * d * d;
}

/// The rattle itself, for a strike of index `sidx` that happened `age` ago.
///
/// Split out from quake_offset because the band pass also fires strikes the
/// clock cannot predict — a window opening — and those have to shake with the
/// same motion. This is the one definition of "what a shake looks like"; only
/// the question of WHEN differs between the callers.
fn quake_shape(amount: f32, age: f32, sidx: f32, index: f32) -> vec2<f32> {
    let env = quake_env(age, 0.85) * amount;
    if (env <= 0.0) {
        return vec2<f32>(0.0, 0.0);
    }
    // Two ringing frequencies per axis so it rattles rather than swings.
    let g = vec2<f32>(
        sin(age * 41.0 + sidx) * 0.7 + sin(age * 67.0 + sidx * 2.3) * 0.3,
        cos(age * 37.0 + sidx * 1.7) * 0.7 + sin(age * 59.0 + sidx) * 0.3,
    );
    let ph = index * 2.399 + sidx * 1.7;
    let r = vec2<f32>(sin(age * 73.0 + ph), sin(age * 61.0 + ph * 1.7));
    return (g * 0.78 + r * 0.22) * env;
}

/// Screen-UV displacement of drawable `index` at time `t`.
///
/// Mostly a GLOBAL jolt — every drawable moves together — with a small
/// per-drawable rattle on top. That split is not decoration: a window's
/// sub-surfaces and popups are separate entries with separate indices, so a
/// purely per-index shake would visibly tear them off the window they belong to.
fn quake_offset(amount: f32, period: f32, t: f32, index: f32) -> vec2<f32> {
    let s = strike_at(t, period);
    return quake_shape(amount, t - s.x, s.y, index);
}

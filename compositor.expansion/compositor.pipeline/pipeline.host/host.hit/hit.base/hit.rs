//! Evaluate a bundle's `hit_inverse` WGSL function on the CPU, over naga's IR.
//!
//! A displacing shader — CRT barrel, lens, ripple, per-window transform — decides
//! per pixel WHERE IT SAMPLED FROM. That is already the function the pointer
//! needs: screen position → the position the content came from. So the bundle
//! writes it once, its render pass `#import`s it, and this interprets the very
//! same source. There is no second implementation to drift.
//!
//! # Why interpret rather than enumerate
//!
//! The obvious alternative is a registry of named warps (`barrel`, `ripple`) with
//! a Rust twin each. That is two implementations of every effect, and the failure
//! mode of drift is a pointer that is subtly wrong — near-unfalsifiable by eye and
//! impossible to unit-test against the shader. Interpreting the author's own
//! function costs an evaluator once instead of a twin per effect, and puts no
//! ceiling on what a bundle may express.
//!
//! # Why the function takes everything as parameters
//!
//! `fn hit_inverse(uv, res, params) -> vec2<f32>` — no globals, no bindings, no
//! textures. That is a deliberate restriction, not an omission: modelling the
//! uniform/push environment would drag the descriptor ABI into an interpreter,
//! and sampling a texture would make the result depend on GPU state this side
//! cannot see. Everything the warp needs is passed in, which is also exactly what
//! lets the render pass call the identical function with its own push constants.
//!
//! # Cost
//!
//! Interpreted per pointer event. A CRT barrel is ~40 IR ops — microseconds at
//! 1 kHz polling. A bundle that writes something genuinely heavy pays for it on
//! the input path, which is the trade: correctness and freedom over a fixed menu.

use compositor_pipeline_host_eval_base::eval;
use naga::{Expression, Handle, Module, Statement};

/// A parsed, validated warp: the module plus the handles of whichever entry
/// functions it defines.
///
/// The two entries are the two kinds in the chain, and a bundle may define either
/// or both:
///
/// * [`ENTRY`] — `hit_inverse(uv, res, params)`, a pure function of position. Being
///   pure is what makes it GRIDDABLE: the engine bakes it into a map once and the
///   pointer costs a lookup (`shader.map`).
/// * [`ENTRY_DRAWABLE`] — `hit_drawable(uv, res, params, rect)`, which additionally
///   sees one world drawable. Per-drawable displacement is DISCONTINUOUS at every
///   drawable edge, so no grid resolution represents it — that error is not a
///   sampling error — and it is evaluated pointwise instead.
///
/// Which kind a function is falls out of its arity; nothing is declared twice and
/// nothing can be mislabelled.
pub struct Warp {
    module: Module,
    func: Option<Handle<naga::Function>>,
    drawable: Option<Handle<naga::Function>>,
}

/// Everything the warp is allowed to know, passed by value. Defined by the
/// evaluator, re-exported here because this is the crate a caller uses.
pub use compositor_pipeline_host_eval_base::eval::Args;

/// The griddable entry: `fn hit_inverse(uv, res, params) -> vec2<f32>`, or
/// `fn hit_inverse(uv, res, params, time) -> vec2<f32>` when it animates.
///
/// The clock is opt-in by ARITY, like everything else here. Taking it makes the
/// function a different thing — a field that is only valid for one instant — which
/// is why `map_static` is refused for a warp that does: a bake of one instant is
/// wrong every frame after it.
pub const ENTRY: &str = "hit_inverse";

/// The per-drawable entry: `fn hit_drawable(uv, res, params, rect, attrs) -> vec3<f32>`,
/// or `fn hit_drawable(uv, res, params, rect, attrs, flags) -> vec3<f32>` when it
/// cares what the compositor knows about the window.
///
/// `rect` is one world drawable's screen-UV rect (xy origin, zw size), handed in
/// front-to-back order, and `attrs` is `vec4(index, time, kind, alpha)` — the same
/// index the pass looped with and the same clock it was pushed for, because a
/// transform built from either cannot be undone without them. `.xy` of the result
/// is the source UV; `.z > 0.5` claims the point, and the first claim wins. A
/// bundle that transforms windows implements this; the engine does the iteration.
///
/// `flags` is the `window.descriptor` bit set — the SAME value the render pass
/// reads from `windows.attrs[i].z`, so a warp and the picture it inverts agree
/// about which window is focused or resizing. It is a sixth ARGUMENT rather than a
/// fifth lane of `attrs` because `attrs` has no free lane and its four are
/// documented, shipped and implemented by an example; renaming them to make room
/// would silently repoint every existing `hit_drawable` at different values. Opt-in
/// by arity, like the clock on [`ENTRY`].
pub const ENTRY_DRAWABLE: &str = "hit_drawable";

/// Parse `src` (a naga_oil module source) and validate that [`ENTRY`] is
/// interpretable.
///
/// `#`-prefixed lines are stripped first: `#define_import_path` and `#import` are
/// naga_oil preprocessor directives that plain naga does not accept, and a warp
/// module is self-contained by construction, so dropping them is lossless here.
///
/// Errors are returned rather than logged so `load_pipeline` can refuse the whole
/// bundle. A warp that silently fell back to identity would be a pointer that is
/// wrong only where the effect is strongest.
pub fn parse(src: &str) -> Result<Warp, String> {
    let stripped: String = src
        .lines()
        .map(|l| if l.trim_start().starts_with('#') { "" } else { l })
        .collect::<Vec<_>>()
        .join("\n");
    let module = naga::front::wgsl::parse_str(&stripped)
        .map_err(|e| format!("hit warp: {}", e.message()))?;
    let find = |name: &str| {
        module.functions.iter().find(|(_, f)| f.name.as_deref() == Some(name)).map(|(h, _)| h)
    };
    let (func, drawable) = (find(ENTRY), find(ENTRY_DRAWABLE));
    if func.is_none() && drawable.is_none() {
        return Err(format!("hit warp: the module defines neither `{ENTRY}` nor `{ENTRY_DRAWABLE}`"));
    }
    let w = Warp { module, func, drawable };
    w.validate()?;
    Ok(w)
}

impl Warp {
    /// Reject anything the evaluator cannot honour, at LOAD time.
    ///
    /// The point is that an unsupported construct fails the bundle with a message
    /// naming it, rather than producing a plausible-looking wrong number at
    /// runtime. Everything rejected here is rejected because it would need state
    /// this side does not have (a binding, a derivative, a neighbouring
    /// invocation), not because it was inconvenient.
    fn validate(&self) -> Result<(), String> {
        for (h, name, arity) in [
            (self.func, ENTRY, &[3usize, 4][..]),
            (self.drawable, ENTRY_DRAWABLE, &[5, 6][..]),
        ] {
            let Some(h) = h else { continue };
            self.validate_one(&self.module.functions[h], name, arity)?;
        }
        Ok(())
    }

    fn validate_one(&self, f: &naga::Function, name: &str, arity: &[usize]) -> Result<(), String> {
        if !arity.contains(&f.arguments.len()) {
            return Err(format!(
                "hit warp: `{name}` takes {} arguments; expected {}",
                f.arguments.len(),
                arity.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" or ")
            ));
        }
        // Two passes, most-specific first. A texture binding is ALSO a global, and
        // reporting it as "reads a global" would send the author looking for a
        // variable when the real answer is that no CPU can sample their texture.
        let specific = |e: &Expression| match e {
            Expression::ImageSample { .. } | Expression::ImageLoad { .. }
            | Expression::ImageQuery { .. } => Some("samples a texture"),
            Expression::Derivative { .. } => Some("takes a derivative"),
            Expression::AtomicResult { .. } => Some("uses an atomic"),
            _ => None,
        };
        let refuse = |what: &str| {
            Err(format!("hit warp: `{name}` {what}, which the CPU evaluator cannot do"))
        };
        if let Some(what) = f.expressions.iter().find_map(|(_, e)| specific(e)) {
            return refuse(what);
        }
        if f.expressions.iter().any(|(_, e)| matches!(e, Expression::GlobalVariable(_))) {
            return refuse("reads a global (pass it as an argument instead)");
        }
        Self::check_block(&f.body)
    }

    fn check_block(b: &naga::Block) -> Result<(), String> {
        for s in b.iter() {
            match s {
                Statement::Emit(_) | Statement::Return { .. } | Statement::Store { .. }
                | Statement::Break | Statement::Continue
                // A call into a helper the bundle shares with its render pass. The
                // evaluator follows it, depth-bounded. Refusing it would force the
                // author to COPY the expression into the warp, which is exactly how
                // the picture and the cursor drift apart.
                | Statement::Call { .. } => {}
                Statement::Block(inner) => Self::check_block(inner)?,
                Statement::If { accept, reject, .. } => {
                    Self::check_block(accept)?;
                    Self::check_block(reject)?;
                }
                Statement::Loop { body, continuing, .. } => {
                    Self::check_block(body)?;
                    Self::check_block(continuing)?;
                }
                _ => return Err("hit warp: unsupported statement".to_string()),
            }
        }
        Ok(())
    }

    /// Evaluate the warp for one point. `uv` and the result are screen UV.
    ///
    /// Returns `None` if evaluation hits something it cannot do — a caller should
    /// treat that as identity for this point rather than dropping the event.
    /// `validate` makes it unlikely, but an evaluator that returns `None` instead
    /// of a guess is the difference between a pointer that is occasionally
    /// un-warped and one that is occasionally somewhere random.
    pub fn eval(&self, uv: (f64, f64), args: Args) -> Option<(f64, f64)> {
        let f = self.func?;
        let mut a = vec![
            eval::Val::vec2(uv.0, uv.1),
            eval::Val::vec2(args.res[0] as f64, args.res[1] as f64),
            eval::Val::vec4(args.params),
        ];
        if self.animated() {
            a.push(eval::Val::scalar(args.time as f64));
        }
        let v = eval::call(&self.module, f, &a)?;
        (v.n >= 2).then_some((v.v[0], v.v[1]))
    }

    /// Whether the griddable entry reads the clock — i.e. takes the optional
    /// fourth argument. A warp that does cannot be served by a static bake.
    pub fn animated(&self) -> bool {
        self.func.is_some_and(|h| self.module.functions[h].arguments.len() == 4)
    }

    /// Whether this bundle defines the griddable entry.
    pub fn has_map(&self) -> bool {
        self.func.is_some()
    }

    /// Whether this bundle defines the per-drawable entry.
    pub fn has_drawable(&self) -> bool {
        self.drawable.is_some()
    }

    /// Whether the per-drawable entry reads the window descriptors — i.e. takes
    /// the optional sixth argument.
    pub fn descriptive(&self) -> bool {
        self.drawable.is_some_and(|h| self.module.functions[h].arguments.len() == 6)
    }

    /// Evaluate the per-drawable entry for one drawable.
    ///
    /// `rect` is its screen-UV rect. `Some((u, v))` means this drawable CLAIMS the
    /// point and the source is `(u, v)`; `None` means it does not, and the caller
    /// tries the next one. The engine owns the iteration order (front-to-back, so
    /// the first claim is the topmost), which keeps every bundle from
    /// re-implementing it and getting the z wrong.
    pub fn eval_drawable(
        &self,
        uv: (f64, f64),
        args: Args,
        rect: [f32; 4],
        attrs: [f32; 4],
        flags: u32,
    ) -> Option<(f64, f64)> {
        let mut a = vec![
            eval::Val::vec2(uv.0, uv.1),
            eval::Val::vec2(args.res[0] as f64, args.res[1] as f64),
            eval::Val::vec4(args.params),
            eval::Val::vec4(rect),
            eval::Val::vec4(attrs),
        ];
        if self.descriptive() {
            a.push(eval::Val::scalar(flags as f64));
        }
        let v = eval::call(&self.module, self.drawable?, &a)?;
        (v.n >= 3 && v.v[2] > 0.5).then_some((v.v[0], v.v[1]))
    }
}

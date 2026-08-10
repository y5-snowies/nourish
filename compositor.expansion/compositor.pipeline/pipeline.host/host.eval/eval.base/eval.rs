//! A small CPU evaluator for a restricted slice of naga IR.
//!
//! Exists so a bundle's pointer-warp function can be run host-side without a
//! second implementation of it — see `shader.hit` for why that matters. This
//! crate is the machinery; `shader.hit` owns the contract and the load-time
//! validation that keeps a function inside the subset below.
//!
//! # The subset, and why it stops where it does
//!
//! Float scalars and vectors, the WGSL math builtins, comparisons, `let`, local
//! variables, `if`/`else`, and loops. Not: textures, derivatives, atomics,
//! globals, or calls to other functions. Each of those needs state that only
//! exists on the GPU or in a descriptor set, so the boundary is "what a pure
//! function of its arguments can compute", not an arbitrary stopping point.
//!
//! Integers are carried as `f64` and converted at the `As` boundary. A warp does
//! geometry, so this loses nothing real and avoids a second numeric tower; a
//! function that wants exact integer semantics is outside the subset anyway.
//!
//! # Termination
//!
//! Loops are bounded by [`MAX_STEPS`] rather than trusted. This runs on the input
//! path, and a bundle is a file a user can edit — an unbounded loop would hang
//! the compositor's pointer, so an evaluation that runs too long returns `None`
//! (treated as identity) instead.

use naga::{
    BinaryOperator, Expression, Function, Handle, Literal, MathFunction, Module, ScalarKind,
    Statement, UnaryOperator,
};
use std::collections::HashMap;

/// Lift a `u32` operation onto the `f64` lanes values are carried in.
///
/// Bitwise operators are the one family whose operands are integers by
/// definition, and a shader spells the integer as a float everywhere else, so the
/// conversion belongs here rather than at each call. Out-of-range operands
/// saturate rather than wrap, which is what `as` does and is the answer that stays
/// closest to the shader's.
fn bits(f: impl Fn(u32, u32) -> u32) -> impl Fn(f64, f64) -> f64 {
    move |x, y| f(x as u32, y as u32) as f64
}

/// Everything the warp is allowed to know, passed by value. No bindings, no
/// globals — see `shader.hit`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Args {
    /// Output resolution in physical pixels.
    pub res: [f32; 2],
    /// The clock the background pass was pushed for. Passed to a warp that takes
    /// the optional fourth argument; ignored by one that does not.
    pub time: f32,
    /// The bundle's first four `@prop` params, so a warp can be driven by the
    /// same tunables the visual effect uses.
    pub params: [f32; 4],
}

/// Interpreter step budget. Generous for any plausible warp (a CRT barrel is
/// ~40), small enough that a runaway costs a frame rather than the session.
pub const MAX_STEPS: u32 = 100_000;

/// A value: a float vector of 1..=4 lanes. `n == 1` is a scalar; bools live here
/// too, as 0.0/1.0, because every use of one is a condition or a `select`.
#[derive(Clone, Copy, Debug)]
pub struct Val {
    pub v: [f64; 4],
    pub n: usize,
}

impl Val {
    /// A one-lane value — a scalar, the clock argument, a bool as 0.0/1.0.
    pub fn scalar(x: f64) -> Self {
        Val { v: [x, 0.0, 0.0, 0.0], n: 1 }
    }
    pub fn vec2(x: f64, y: f64) -> Self {
        Val { v: [x, y, 0.0, 0.0], n: 2 }
    }
    pub fn vec4(a: [f32; 4]) -> Self {
        Val { v: [a[0] as f64, a[1] as f64, a[2] as f64, a[3] as f64], n: 4 }
    }
    fn truthy(self) -> bool {
        self.v[0] != 0.0
    }
    /// Lane-wise apply, broadcasting a scalar against a vector — WGSL's rule for
    /// `vec * f32` and friends.
    fn zip(self, o: Val, f: impl Fn(f64, f64) -> f64) -> Val {
        let n = self.n.max(o.n);
        let mut v = [0.0; 4];
        for (i, slot) in v.iter_mut().enumerate().take(n) {
            let a = if self.n == 1 { self.v[0] } else { self.v[i] };
            let b = if o.n == 1 { o.v[0] } else { o.v[i] };
            *slot = f(a, b);
        }
        Val { v, n }
    }
    fn map(self, f: impl Fn(f64) -> f64) -> Val {
        let mut v = [0.0; 4];
        for (i, slot) in v.iter_mut().enumerate().take(self.n) {
            *slot = f(self.v[i]);
        }
        Val { v, n: self.n }
    }
    fn dot(self, o: Val) -> f64 {
        (0..self.n.max(o.n)).map(|i| self.v[i] * o.v[i]).sum()
    }
    fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
}

/// Where a `Return` landed, or that control flow left the block.
enum Flow {
    Normal,
    Break,
    Continue,
    Return(Option<Val>),
}

struct Ctx<'a> {
    module: &'a Module,
    func: &'a Function,
    args: &'a [Val],
    expr: HashMap<Handle<Expression>, Val>,
    local: HashMap<Handle<naga::LocalVariable>, Val>,
    /// Shared with every callee, so a budget is a budget for the whole evaluation
    /// rather than per frame of a recursion.
    steps: &'a std::cell::Cell<u32>,
    /// Call depth. Bounded because WGSL forbids recursion but a malformed module
    /// need not, and the interpreter must not blow the host stack over one.
    depth: u32,
}

/// Call-depth ceiling. WGSL has no recursion, so anything approaching this is a
/// module the front-end should have rejected.
const MAX_DEPTH: u32 = 32;

/// Run `func` in `module` with `uv` and `args`, returning its `vec2` result.
///
/// `None` on anything outside the subset, on a missing value, or on exceeding
/// [`MAX_STEPS`]. Callers treat `None` as identity rather than as a failure —
/// see `shader.hit::Warp::eval`.
pub fn run(
    module: &Module,
    func: Handle<Function>,
    uv: (f64, f64),
    args: Args,
) -> Option<(f64, f64)> {
    let v = call(
        module,
        func,
        &[
            Val::vec2(uv.0, uv.1),
            Val::vec2(args.res[0] as f64, args.res[1] as f64),
            Val::vec4(args.params),
        ],
    )?;
    (v.n >= 2).then_some((v.v[0], v.v[1]))
}

/// Run `func` with an arbitrary argument list, returning its value.
///
/// The general form. `run` is the two-argument warp; a per-drawable entry passes
/// the drawable's rect as a fourth and reads three lanes back, and neither shape
/// is special to the interpreter — only the caller knows what the lanes mean.
pub fn call(module: &Module, func: Handle<Function>, args: &[Val]) -> Option<Val> {
    let steps = std::cell::Cell::new(0);
    invoke(module, func, args, &steps, 0)
}

fn invoke(
    module: &Module,
    func: Handle<Function>,
    args: &[Val],
    steps: &std::cell::Cell<u32>,
    depth: u32,
) -> Option<Val> {
    if depth > MAX_DEPTH {
        return None;
    }
    let f = &module.functions[func];
    let mut cx = Ctx {
        module,
        func: f,
        args,
        expr: HashMap::new(),
        local: HashMap::new(),
        steps,
        depth,
    };
    match cx.block(&f.body)? {
        Flow::Return(Some(v)) => Some(v),
        _ => None,
    }
}

impl Ctx<'_> {
    fn tick(&mut self) -> Option<()> {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        (n < MAX_STEPS).then_some(())
    }

    fn block(&mut self, b: &naga::Block) -> Option<Flow> {
        for s in b.iter() {
            self.tick()?;
            match s {
                Statement::Emit(range) => {
                    for h in range.clone() {
                        let v = self.expr(h)?;
                        self.expr.insert(h, v);
                    }
                }
                Statement::Block(inner) => match self.block(inner)? {
                    Flow::Normal => {}
                    other => return Some(other),
                },
                Statement::If { condition, accept, reject } => {
                    let c = self.expr(*condition)?;
                    let taken = if c.truthy() { accept } else { reject };
                    match self.block(taken)? {
                        Flow::Normal => {}
                        other => return Some(other),
                    }
                }
                Statement::Loop { body, continuing, break_if } => loop {
                    self.tick()?;
                    match self.block(body)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Some(Flow::Return(v)),
                        Flow::Normal | Flow::Continue => {}
                    }
                    match self.block(continuing)? {
                        Flow::Return(v) => return Some(Flow::Return(v)),
                        _ => {}
                    }
                    if let Some(c) = break_if {
                        if self.expr(*c)?.truthy() {
                            break;
                        }
                    }
                },
                // A helper the bundle shares with its render pass. Supporting this
                // is what lets one definition have two readers — the alternative is
                // the author copying the expression into the warp, which is exactly
                // how the picture and the cursor drift apart.
                Statement::Call { function, arguments, result } => {
                    let mut vals = Vec::with_capacity(arguments.len());
                    for a in arguments {
                        vals.push(self.expr(*a)?);
                    }
                    let v = invoke(self.module, *function, &vals, self.steps, self.depth + 1)?;
                    if let Some(r) = result {
                        self.expr.insert(*r, v);
                    }
                }
                Statement::Break => return Some(Flow::Break),
                Statement::Continue => return Some(Flow::Continue),
                Statement::Return { value } => {
                    let v = match value {
                        Some(h) => Some(self.expr(*h)?),
                        None => None,
                    };
                    return Some(Flow::Return(v));
                }
                Statement::Store { pointer, value } => {
                    let v = self.expr(*value)?;
                    // Only a direct local is a valid store target here; anything
                    // else needs a pointer model this subset does not have.
                    let Expression::LocalVariable(l) = self.func.expressions[*pointer] else {
                        return None;
                    };
                    self.local.insert(l, v);
                }
                _ => return None,
            }
        }
        Some(Flow::Normal)
    }

    fn expr(&mut self, h: Handle<Expression>) -> Option<Val> {
        if let Some(v) = self.expr.get(&h) {
            return Some(*v);
        }
        self.tick()?;
        let v = match &self.func.expressions[h] {
            Expression::Literal(l) => Val::scalar(match *l {
                Literal::F64(x) => x,
                Literal::F32(x) => x as f64,
                Literal::I32(x) => x as f64,
                Literal::U32(x) => x as f64,
                Literal::I64(x) => x as f64,
                Literal::U64(x) => x as f64,
                Literal::AbstractInt(x) => x as f64,
                Literal::AbstractFloat(x) => x,
                Literal::Bool(b) => b as i32 as f64,
                _ => return None,
            }),
            Expression::ZeroValue(_) => Val::scalar(0.0),
            Expression::FunctionArgument(i) => *self.args.get(*i as usize)?,
            Expression::LocalVariable(_) => Val::scalar(0.0), // resolved by Load/Store
            Expression::Load { pointer } => {
                let Expression::LocalVariable(l) = self.func.expressions[*pointer] else {
                    return None;
                };
                *self.local.get(&l).unwrap_or(&Val::scalar(0.0))
            }
            Expression::Compose { components, .. } => {
                let mut v = [0.0; 4];
                let mut n = 0;
                for c in components {
                    let cv = self.expr(*c)?;
                    for i in 0..cv.n {
                        if n < 4 {
                            v[n] = cv.v[i];
                            n += 1;
                        }
                    }
                }
                Val { v, n: n.max(1) }
            }
            Expression::Splat { size, value } => {
                let s = self.expr(*value)?.v[0];
                Val { v: [s; 4], n: *size as usize }
            }
            Expression::Swizzle { size, vector, pattern } => {
                let src = self.expr(*vector)?;
                let mut v = [0.0; 4];
                for (i, slot) in v.iter_mut().enumerate().take(*size as usize) {
                    *slot = src.v[pattern[i] as usize];
                }
                Val { v, n: *size as usize }
            }
            Expression::AccessIndex { base, index } => {
                let b = self.expr(*base)?;
                Val::scalar(*b.v.get(*index as usize)?)
            }
            Expression::Access { base, index } => {
                let b = self.expr(*base)?;
                let i = self.expr(*index)?.v[0] as usize;
                Val::scalar(*b.v.get(i)?)
            }
            Expression::Unary { op, expr } => {
                let a = self.expr(*expr)?;
                match op {
                    UnaryOperator::Negate => a.map(|x| -x),
                    UnaryOperator::LogicalNot => Val::scalar((a.v[0] == 0.0) as i32 as f64),
                    _ => return None,
                }
            }
            Expression::Binary { op, left, right } => {
                let (a, b) = (self.expr(*left)?, self.expr(*right)?);
                let bool_of = |x: bool| x as i32 as f64;
                match op {
                    BinaryOperator::Add => a.zip(b, |x, y| x + y),
                    BinaryOperator::Subtract => a.zip(b, |x, y| x - y),
                    BinaryOperator::Multiply => a.zip(b, |x, y| x * y),
                    BinaryOperator::Divide => a.zip(b, |x, y| x / y),
                    BinaryOperator::Modulo => a.zip(b, |x, y| x % y),
                    BinaryOperator::Equal => a.zip(b, move |x, y| bool_of(x == y)),
                    BinaryOperator::NotEqual => a.zip(b, move |x, y| bool_of(x != y)),
                    BinaryOperator::Less => a.zip(b, move |x, y| bool_of(x < y)),
                    BinaryOperator::LessEqual => a.zip(b, move |x, y| bool_of(x <= y)),
                    BinaryOperator::Greater => a.zip(b, move |x, y| bool_of(x > y)),
                    BinaryOperator::GreaterEqual => a.zip(b, move |x, y| bool_of(x >= y)),
                    BinaryOperator::LogicalAnd => {
                        Val::scalar(bool_of(a.truthy() && b.truthy()))
                    }
                    BinaryOperator::LogicalOr => Val::scalar(bool_of(a.truthy() || b.truthy())),
                    // Bitwise, on the integer a value denotes. Needed because the
                    // window descriptors reach a warp as a BIT SET, and testing a
                    // bit is `(u32(flags) & FLAG) != 0u` — the same line the render
                    // pass writes. Without these the warp would evaluate to nothing
                    // and the point would go silently unclaimed, which is a pointer
                    // that is wrong in exactly the places the effect is strongest.
                    //
                    // Values are carried as f64, so these are exact over the u32
                    // range the descriptors live in and over anything else a warp
                    // has business masking.
                    BinaryOperator::And => a.zip(b, bits(|x, y| x & y)),
                    BinaryOperator::InclusiveOr => a.zip(b, bits(|x, y| x | y)),
                    BinaryOperator::ExclusiveOr => a.zip(b, bits(|x, y| x ^ y)),
                    BinaryOperator::ShiftLeft => a.zip(b, bits(|x, y| x.wrapping_shl(y))),
                    BinaryOperator::ShiftRight => a.zip(b, bits(|x, y| x.wrapping_shr(y))),
                    _ => return None,
                }
            }
            Expression::Select { condition, accept, reject } => {
                let c = self.expr(*condition)?;
                if c.truthy() { self.expr(*accept)? } else { self.expr(*reject)? }
            }
            // A cast to an INTEGER truncates, as WGSL's does. It used to be a
            // passthrough for every target, which is invisible until something
            // masks the result: `u32(flags) & 1u` on a value that kept a fraction
            // masks the wrong number.
            Expression::As { expr, kind, .. } => {
                let v = self.expr(*expr)?;
                match kind {
                    ScalarKind::Sint | ScalarKind::Uint => v.map(f64::trunc),
                    ScalarKind::Bool => v.map(|x| (x != 0.0) as i32 as f64),
                    _ => v,
                }
            }
            Expression::Constant(c) => {
                let init = self.module.constants[*c].init;
                let e = &self.module.global_expressions[init];
                match e {
                    Expression::Literal(Literal::F32(x)) => Val::scalar(*x as f64),
                    Expression::Literal(Literal::F64(x)) => Val::scalar(*x),
                    Expression::Literal(Literal::I32(x)) => Val::scalar(*x as f64),
                    Expression::Literal(Literal::U32(x)) => Val::scalar(*x as f64),
                    _ => return None,
                }
            }
            Expression::Math { fun, arg, arg1, arg2, .. } => {
                let a = self.expr(*arg)?;
                let b = match arg1 {
                    Some(h) => Some(self.expr(*h)?),
                    None => None,
                };
                let c = match arg2 {
                    Some(h) => Some(self.expr(*h)?),
                    None => None,
                };
                self.math(*fun, a, b, c)?
            }
            _ => return None,
        };
        self.expr.insert(h, v);
        Some(v)
    }

    fn math(&self, f: MathFunction, a: Val, b: Option<Val>, c: Option<Val>) -> Option<Val> {
        use MathFunction as M;
        Some(match f {
            M::Abs => a.map(f64::abs),
            M::Sign => a.map(f64::signum),
            M::Floor => a.map(f64::floor),
            M::Ceil => a.map(f64::ceil),
            M::Round => a.map(|x| x.round()),
            M::Trunc => a.map(f64::trunc),
            M::Fract => a.map(|x| x - x.floor()),
            M::Sqrt => a.map(f64::sqrt),
            M::InverseSqrt => a.map(|x| 1.0 / x.sqrt()),
            M::Exp => a.map(f64::exp),
            M::Exp2 => a.map(f64::exp2),
            M::Log => a.map(f64::ln),
            M::Log2 => a.map(f64::log2),
            M::Sin => a.map(f64::sin),
            M::Cos => a.map(f64::cos),
            M::Tan => a.map(f64::tan),
            M::Asin => a.map(f64::asin),
            M::Acos => a.map(f64::acos),
            M::Atan => a.map(f64::atan),
            M::Sinh => a.map(f64::sinh),
            M::Cosh => a.map(f64::cosh),
            M::Tanh => a.map(f64::tanh),
            M::Radians => a.map(f64::to_radians),
            M::Degrees => a.map(f64::to_degrees),
            M::Saturate => a.map(|x| x.clamp(0.0, 1.0)),
            M::Length => Val::scalar(a.len()),
            M::Normalize => {
                let l = a.len();
                a.map(|x| x / l)
            }
            M::Atan2 => a.zip(b?, f64::atan2),
            M::Pow => a.zip(b?, f64::powf),
            M::Min => a.zip(b?, f64::min),
            M::Max => a.zip(b?, f64::max),
            M::Step => b?.zip(a, |x, e| (x >= e) as i32 as f64),
            M::Dot => Val::scalar(a.dot(b?)),
            M::Distance => Val::scalar(a.zip(b?, |x, y| x - y).len()),
            M::Cross => {
                let o = b?;
                Val {
                    v: [
                        a.v[1] * o.v[2] - a.v[2] * o.v[1],
                        a.v[2] * o.v[0] - a.v[0] * o.v[2],
                        a.v[0] * o.v[1] - a.v[1] * o.v[0],
                        0.0,
                    ],
                    n: 3,
                }
            }
            M::Clamp => {
                let (lo, hi) = (b?, c?);
                a.zip(lo, f64::max).zip(hi, f64::min)
            }
            M::Mix => {
                let (y, t) = (b?, c?);
                let n = a.n.max(y.n).max(t.n);
                let mut v = [0.0; 4];
                for (i, slot) in v.iter_mut().enumerate().take(n) {
                    let (xa, xb) = (lane(a, i), lane(y, i));
                    *slot = xa + (xb - xa) * lane(t, i);
                }
                Val { v, n }
            }
            M::SmoothStep => {
                let (e1, x) = (b?, c?);
                let n = a.n.max(e1.n).max(x.n);
                let mut v = [0.0; 4];
                for (i, slot) in v.iter_mut().enumerate().take(n) {
                    let (e0, e1i, xi) = (lane(a, i), lane(e1, i), lane(x, i));
                    let t = ((xi - e0) / (e1i - e0)).clamp(0.0, 1.0);
                    *slot = t * t * (3.0 - 2.0 * t);
                }
                Val { v, n }
            }
            M::Fma => {
                let (y, z) = (b?, c?);
                a.zip(y, |x, yy| x * yy).zip(z, |x, zz| x + zz)
            }
            _ => return None,
        })
    }
}

fn lane(v: Val, i: usize) -> f64 {
    if v.n == 1 { v.v[0] } else { v.v[i] }
}

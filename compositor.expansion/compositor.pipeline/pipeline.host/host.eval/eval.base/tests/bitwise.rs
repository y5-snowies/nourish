//! Integer arithmetic, which the evaluator gained because the window descriptors
//! reach a warp as a bit set.
//!
//! A shader tests one with `(u32(flags) & FLAG) != 0u`. Before these operators
//! that expression evaluated to nothing, and an unevaluable warp returns `None` —
//! which the caller treats as "this drawable does not claim the point". So the
//! failure was not an error anywhere: the pointer simply stopped being corrected,
//! and only under the bundles that looked at a descriptor.

use compositor_pipeline_host_eval_base::eval::{self, Val};

/// Evaluate `fn f(a: f32, b: f32) -> f32` over the given operands.
fn run(body: &str, a: f64, b: f64) -> Option<f64> {
    let src = format!("fn f(a: f32, b: f32) -> f32 {{ {body} }}");
    let m = naga::front::wgsl::parse_str(&src).expect("parses");
    let h = m.functions.iter().find(|(_, f)| f.name.as_deref() == Some("f")).map(|(h, _)| h)?;
    let v = eval::call(&m, h, &[Val::scalar(a), Val::scalar(b)])?;
    Some(v.v[0])
}

/// The expression a bundle actually writes, both ways round.
#[test]
fn a_flag_test_evaluates() {
    const TEST: &str = "if ((u32(a) & u32(b)) != 0u) { return 1.0; } return 0.0;";
    assert_eq!(run(TEST, 0b1011u32 as f64, 0b0010u32 as f64), Some(1.0), "a set bit must read as set");
    assert_eq!(run(TEST, 0b1011u32 as f64, 0b0100u32 as f64), Some(0.0), "a clear bit must read as clear");
    // The bit that is set in neither operand and the empty set: the two ways a
    // mask can be trivially wrong without the arithmetic being wrong.
    assert_eq!(run(TEST, 0.0, 0b0001u32 as f64), Some(0.0));
    assert_eq!(run(TEST, 0b1111u32 as f64, 0.0), Some(0.0));
}

/// The rest of the family, since a warp masking bits will shift them too.
#[test]
fn the_integer_operators_agree_with_rust() {
    for (body, a, b, want) in [
        ("return f32(u32(a) & u32(b));", 0b1100u32 as f64, 0b1010u32 as f64, 0b1000u32 as f64),
        ("return f32(u32(a) | u32(b));", 0b1100u32 as f64, 0b1010u32 as f64, 0b1110u32 as f64),
        ("return f32(u32(a) ^ u32(b));", 0b1100u32 as f64, 0b1010u32 as f64, 0b0110u32 as f64),
        ("return f32(u32(a) << u32(b));", 3.0, 4.0, 48.0),
        ("return f32(u32(a) >> u32(b));", 48.0, 4.0, 3.0),
    ] {
        assert_eq!(run(body, a, b), Some(want), "{body}");
    }
}

/// A cast to an integer TRUNCATES. It used to be a passthrough, which is
/// invisible until something masks the result — `u32(1.9) & 1u` is 1, but only if
/// the cast actually happened.
#[test]
fn an_integer_cast_truncates() {
    assert_eq!(run("return f32(u32(a));", 1.9, 0.0), Some(1.0));
    assert_eq!(run("return f32(i32(a));", -1.9, 0.0), Some(-1.0), "toward zero, as WGSL does");
    assert_eq!(run("return f32(u32(a) & 1u);", 3.7, 0.0), Some(1.0));
    // …and a float cast still does not, or every warp's arithmetic changes.
    assert_eq!(run("return f32(a);", 1.9, 0.0), Some(1.9));
}

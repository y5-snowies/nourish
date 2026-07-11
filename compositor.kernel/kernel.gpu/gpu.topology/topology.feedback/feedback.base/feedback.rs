//! Per-surface dmabuf-feedback modifier policy (multi-primary).
//!
//! A Wayland client renders a surface into ONE dmabuf; the compositor imports that
//! buffer into every GPU that composites an output the surface is on. So the buffer's
//! modifier must be importable by ALL those GPUs. This computes the modifier set to
//! advertise to the client: the **intersection** of each showing-GPU's importable
//! modifiers, with `LINEAR` always included as the universal interop floor (every
//! importer handles linear, so even a cross-vendor empty intersection has a working
//! option). A surface on a single GPU therefore gets that GPU's full (best) set; a
//! surface spanning GPUs on different cards narrows to the common set (+ LINEAR).
//!
//! This is the modifier half of Stage-5 per-surface feedback; the caller maps the
//! result into a `DmabufFeedbackBuilder` tranche (scanout tranche for the surface's
//! predominant GPU, import tranche for the rest) and calls
//! `SurfaceDmabufFeedbackState::set_feedback`.

use smithay::backend::allocator::Modifier;

/// The modifiers to advertise for a fourcc to a surface shown on the given GPUs,
/// where `per_gpu[i]` is GPU *i*'s importable modifier list for that fourcc.
///
/// Result = ∩(per_gpu) ∪ {LINEAR}. `LINEAR` is appended whenever it is not already
/// in the intersection so the floor is always offered. No GPUs → `[LINEAR]`.
pub fn advertise(per_gpu: &[Vec<Modifier>]) -> Vec<Modifier> {
    let mut out: Vec<Modifier> = match per_gpu.split_first() {
        None => Vec::new(),
        Some((first, rest)) => first
            .iter()
            .copied()
            .filter(|m| rest.iter().all(|g| g.contains(m)))
            .collect(),
    };
    if !out.contains(&Modifier::Linear) {
        out.push(Modifier::Linear);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Arbitrary distinct non-linear modifiers (opaque values); LINEAR is `from(0)`.
    fn m(v: u64) -> Modifier {
        Modifier::from(v)
    }
    fn a() -> Modifier {
        m(1)
    }
    fn b() -> Modifier {
        m(2)
    }
    fn c() -> Modifier {
        m(3)
    }
    fn l() -> Modifier {
        Modifier::Linear
    }

    #[test]
    fn no_gpus_offers_only_linear() {
        assert_eq!(advertise(&[]), vec![l()]);
    }

    #[test]
    fn single_gpu_keeps_its_whole_set() {
        assert_eq!(advertise(&[vec![a(), b(), l()]]), vec![a(), b(), l()]);
    }

    #[test]
    fn single_gpu_without_linear_gets_linear_appended() {
        assert_eq!(advertise(&[vec![a(), b()]]), vec![a(), b(), l()]);
    }

    #[test]
    fn spanning_gpus_intersect() {
        // b is common to both; a and c are not. LINEAR common → kept once.
        assert_eq!(advertise(&[vec![a(), b(), l()], vec![b(), c(), l()]]), vec![b(), l()]);
    }

    #[test]
    fn empty_intersection_falls_to_linear_floor() {
        // Cross-vendor: no shared tiled modifier, none listed linear → LINEAR is
        // still offered as the universal floor.
        assert_eq!(advertise(&[vec![a()], vec![c()]]), vec![l()]);
    }
}

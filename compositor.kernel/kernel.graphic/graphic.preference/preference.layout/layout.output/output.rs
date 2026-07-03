//! Global-space output placement. Backends produce outputs; the compositor places
//! them. Because y5 renders each monitor through its own camera (not one extended
//! coordinate space), these global positions are bookkeeping only — they must merely
//! be non-overlapping so smithay's `Space`, pointer routing and layer-shell math stay
//! consistent. Horizontal tiling by mode width is the simplest such arrangement.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputPosition(pub i32, pub i32);

/// Single-output / legacy placement: the origin.
pub fn position_for(_identity: Option<&str>, _index: usize) -> OutputPosition {
    OutputPosition(0, 0)
}

/// Lay N outputs left-to-right at `y = 0`: each output's `x` is the running sum of
/// all prior outputs' widths, so they tile without overlap. `widths` is in the
/// caller's stable connector order; the result is 1:1 with it. Pure + total (empty
/// in → empty out; a zero/negative width contributes no advance).
pub fn tile_positions(widths: &[i32]) -> Vec<OutputPosition> {
    let mut x = 0;
    widths
        .iter()
        .map(|&w| {
            let pos = OutputPosition(x, 0);
            x += w.max(0);
            pos
        })
        .collect()
}

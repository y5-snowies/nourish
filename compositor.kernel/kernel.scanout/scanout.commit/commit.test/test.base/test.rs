//! TEST_ONLY validation with a mode fallback chain — the home of the
//! try-modes-in-order pseudocode carried in the original wire.rs comments.
//! Seam: under delegation the validating commit happens inside use_mode /
//! initialize_output; this crate owns the FALLBACK ORDER policy, which is
//! real and consumed by assembly.

use smithay::reexports::drm::control::{connector, Mode as DrmMode, ModeFlags, ModeTypeFlags};

/// The fallback chain: candidate modes in the order they should be attempted
/// when a modeset fails — the selected mode first, then by the same
/// area -> refresh -> PREFERRED ordering, deduplicated.
pub fn fallback_chain(info: &connector::Info, selected: DrmMode) -> Vec<DrmMode> {
    let mut chain: Vec<DrmMode> = vec![selected];
    let mut rest: Vec<&DrmMode> = info.modes().iter().collect();
    rest.sort_by_key(|m| {
        let (w, h) = m.size();
        // Progressive ahead of interlaced, before anything else: an interlaced
        // candidate cannot pass the atomic test with a tiled buffer, so trying one
        // early just burns a modeset attempt AND risks smithay's bandwidth
        // escalation dropping every pipe to implicit modifiers.
        std::cmp::Reverse((
            !m.flags().contains(ModeFlags::INTERLACE),
            (w as u64) * (h as u64),
            m.vrefresh(),
            m.mode_type().contains(ModeTypeFlags::PREFERRED) as u8,
        ))
    });
    for m in rest {
        if !chain.iter().any(|c| c == m) {
            chain.push(*m);
        }
    }
    chain
}

/// Walk the chain with a caller-provided attempt (the validating modeset).
/// Returns the mode that succeeded.
pub fn try_chain(
    chain: Vec<DrmMode>,
    mut attempt: impl FnMut(DrmMode) -> Result<(), String>,
) -> Result<DrmMode, String> {
    let mut last_err = String::from("empty mode chain");
    for mode in chain {
        match attempt(mode) {
            Ok(()) => return Ok(mode),
            Err(e) => {
                warn!(
                    "modeset failed for {}x{}, trying next: {e}",
                    mode.size().0,
                    mode.size().1
                );
                last_err = e;
            }
        }
    }
    Err(last_err)
}

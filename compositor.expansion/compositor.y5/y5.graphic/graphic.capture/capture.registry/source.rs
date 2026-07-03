//! What to capture: a full output framebuffer, or a sub-region of one.

use smithay::utils::{Physical, Rectangle};

/// Identifies a compositor output by a STABLE id derived from its identity
/// (EDID key, connector-name fallback) — not a positional index, so it survives
/// hotplug reordering and means the same monitor across the render loop and the
/// capture-request side. ALWAYS build it with [`OutputId::from_key`] from the same
/// `output_key` the rim uses (every backend, incl. single-output / winit) so the
/// producer and consumer ids match; never key an output by its positional index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputId(pub u64);

impl OutputId {
    /// Stable id for an output's identity string (its `output_key`: "make model
    /// serial", or the connector name when EDID is unavailable). FNV-1a 64-bit —
    /// deterministic and dependency-free, so the kernel render loop and the rim
    /// capture requests derive the SAME id for the same monitor without sharing
    /// any state or a positional index.
    pub fn from_key(key: &str) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in key.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        OutputId(hash)
    }
}

/// What this capture reads from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSource {
    /// The full framebuffer of an output. The registry blits from the
    /// caller-provided framebuffer in `tick`.
    OutputFramebuffer(OutputId),
    /// A sub-region of an output's framebuffer, in physical/output pixels.
    ///
    /// The entry/dmabuf is sized to `rect.size`; the blit copies the
    /// `rect` sub-rect of the composed scene into it. `rect` is mutable over
    /// the capture's lifetime via [`crate::CaptureRegistry::set_region`] — a
    /// move that keeps the same size reuses the entry (only the blit src
    /// changes); a size change reallocates in place (the `EntryId` stays
    /// stable).
    Region {
        output: OutputId,
        rect: Rectangle<i32, Physical>,
    },
}

impl CaptureSource {
    /// The output this source reads from.
    pub fn output(&self) -> OutputId {
        match self {
            CaptureSource::OutputFramebuffer(o) => *o,
            CaptureSource::Region { output, .. } => *output,
        }
    }

    /// The source sub-rect within the output framebuffer, if this source is
    /// a region. `None` means "the whole framebuffer" (the caller supplies
    /// the framebuffer size as the src rect).
    pub fn src_rect(&self) -> Option<Rectangle<i32, Physical>> {
        match self {
            CaptureSource::OutputFramebuffer(_) => None,
            CaptureSource::Region { rect, .. } => Some(*rect),
        }
    }
}

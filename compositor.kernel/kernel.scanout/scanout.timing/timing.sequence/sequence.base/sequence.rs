//! The CRTC's current vblank sequence (MSC), asked of the kernel directly.
//!
//! The normal source is the page-flip completion event's `frame` field, which arrives
//! through `DrmEventMetadata::sequence` and costs nothing. This exists because that
//! field is not always populated: some drivers deliver the flip event with `frame = 0`
//! on every frame, which makes every client-visible MSC delta zero and so makes a
//! stuttering run indistinguishable from a smooth one.
//!
//! `DRM_IOCTL_CRTC_GET_SEQUENCE` answers the same question from the CRTC itself rather
//! than from the event, so it is unaffected by however the flip path fills that struct.
//! It is a per-CRTC query taking a raw crtc id, which is why this is used rather than
//! `drmWaitVBlank` — the latter identifies the pipe by index and needs the high-crtc
//! bits worked out, and a wrong index silently answers about a different output.
//!
//! Cheap enough to sit on the vblank path: one ioctl per completed flip, not per frame
//! of client work.

use std::os::fd::{AsRawFd, BorrowedFd};

/// `struct drm_crtc_get_sequence` (include/uapi/drm/drm.h). Field order and widths are
/// ABI — do not reorder.
#[repr(C)]
struct GetSequence {
    crtc_id: u32,
    /// Whether vblank is currently enabled on this CRTC. A sequence read while inactive
    /// is not meaningful, so it is reported rather than silently returned.
    active: u32,
    sequence: u64,
    sequence_ns: i64,
}

/// `DRM_IOCTL_CRTC_GET_SEQUENCE` = `_IOWR('d', 0x3b, struct drm_crtc_get_sequence)`:
/// dir=3 (read|write), size=24, type=0x64, nr=0x3b.
const REQ: libc::c_ulong = 0xc018_643b;

/// The request number above encodes the struct's SIZE, so a layout mistake would send a
/// well-formed ioctl for a different command rather than fail. Pin it.
const _: () = assert!(core::mem::size_of::<GetSequence>() == 24);

/// The CRTC's current sequence, or `None` when the ioctl is unsupported or the CRTC has
/// vblank disabled. `None` means "no better answer than the caller already has" — it is
/// never a reason to fail a flip.
pub fn current(fd: BorrowedFd<'_>, crtc_id: u32) -> Option<u64> {
    let mut arg = GetSequence { crtc_id, active: 0, sequence: 0, sequence_ns: 0 };
    // SAFETY: the ioctl writes only into `arg`, whose layout matches the uapi struct.
    let rc = unsafe { libc::ioctl(fd.as_raw_fd(), REQ, &mut arg as *mut GetSequence) };
    (rc == 0 && arg.active != 0).then_some(arg.sequence)
}

/// The sequence to report for a completed flip: the event's own value when it carries
/// one, else the CRTC's.
///
/// Zero is the tell. MSC counts retraces since the CRTC was enabled and only ever
/// advances, so a flip completing with 0 means the field was not filled in — not that
/// no frame has been scanned out, because a flip completing IS a frame. Treating 0 as
/// "ask the CRTC" therefore costs one ioctl on a driver that reports properly (only
/// ever the genuine first frame) and repairs the value on one that does not.
///
/// Why it matters: clients use MSC for what timestamps cannot express — a jump of 2
/// means a dropped frame. A constant 0 makes every delta 0, so a stuttering session is
/// indistinguishable from a smooth one, and frame pacing that reads MSC (Chrome's does)
/// has nothing to pace against.
///
/// Reports which source is in use ONCE per outcome, so a normal run says whether the
/// driver populates the event without anyone having to instrument it again.
pub fn resolve(fd: BorrowedFd<'_>, crtc_id: u32, from_event: u64) -> u64 {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SAID_EVENT: AtomicBool = AtomicBool::new(false);
    static SAID_CRTC: AtomicBool = AtomicBool::new(false);
    static SAID_NONE: AtomicBool = AtomicBool::new(false);

    if from_event != 0 {
        if !SAID_EVENT.swap(true, Ordering::Relaxed) {
            info!("vblank msc: page-flip events carry a sequence ({from_event}); using it");
        }
        return from_event;
    }
    match current(fd, crtc_id) {
        Some(seq) => {
            if !SAID_CRTC.swap(true, Ordering::Relaxed) {
                warn!(
                    "vblank msc: page-flip events report sequence 0; using \
                     DRM_IOCTL_CRTC_GET_SEQUENCE instead (crtc {crtc_id} is at {seq})"
                );
            }
            seq
        }
        None => {
            if !SAID_NONE.swap(true, Ordering::Relaxed) {
                warn!(
                    "vblank msc: page-flip events report sequence 0 AND crtc {crtc_id} has no \
                     queryable sequence — presentation feedback will report 0, so clients \
                     cannot detect dropped frames on this output"
                );
            }
            from_event
        }
    }
}

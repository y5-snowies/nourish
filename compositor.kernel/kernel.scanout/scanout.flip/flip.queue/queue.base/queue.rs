//! queue_frame pacing with the crash-first error policy.
//!
//! Policy (decided, replacing the original's carried "This is temporary"
//! log-and-abort): a queue_frame failure during normal operation is not
//! self-recovering — panic. Two tolerated cases:
//! - session resume before the first real vblank (`resuming = true`): the
//!   kernel may still be handing the device back, the watchdog drives recovery;
//! - a lost-DRM-master failure (EACCES/EPERM): the page flip raced a VT switch
//!   away; the libseat pause event lands a beat later and the resume flow
//!   recovers. Aborting here would crash the compositor on every VT switch.

use compositor_kernel_scanout_surface_output_base::output::{FrameUserData, NativeDrmOutput};

/// True if any error in the chain is an `io::Error` with `PermissionDenied`
/// (EACCES/EPERM) — i.e. we lost DRM master (VT switch / session deactivating).
fn is_lost_drm_master(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cur {
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            if io.kind() == std::io::ErrorKind::PermissionDenied {
                return true;
            }
        }
        cur = e.source();
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueOutcome {
    /// Frame queued; a VBlank will follow.
    Queued,
    /// Queue failed inside the tolerated resume window; the watchdog recovers.
    DeferredToWatchdog,
    /// Queue failed outside the resume window. Multi-output policy: rather than
    /// crash the whole compositor for one connector, the caller tears down THIS
    /// pipe (it goes dark) and the others keep running. The failing pipe is
    /// recovered on the next hotplug reconcile.
    Failed,
}

pub fn queue(
    output: &mut NativeDrmOutput,
    user_data: FrameUserData,
    resuming: bool,
) -> QueueOutcome {
    match output.queue_frame(user_data) {
        Ok(()) => QueueOutcome::Queued,
        Err(err) if resuming => {
            warn!("queue_frame failed during resume window (watchdog recovers): {err:?}");
            QueueOutcome::DeferredToWatchdog
        }
        Err(err) if is_lost_drm_master(&err) => {
            warn!("queue_frame lost DRM master (VT switch / session deactivating); deferring to resume: {err:?}");
            QueueOutcome::DeferredToWatchdog
        }
        Err(err) => {
            warn!("queue_frame failed outside the resume window; tearing down this pipe (others keep running): {err:?}");
            QueueOutcome::Failed
        }
    }
}

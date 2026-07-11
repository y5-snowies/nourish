//! The one sanctioned session fd open/close path. OFlags policy lives here
//! and nowhere else. Returns a plain owned fd (Law 1).
//! Failure policy: failing to open the selected device is not self-recovering
//! — panic (the original unwrapped here too).

use smithay::backend::session::Session;
use smithay::reexports::rustix::fs::OFlags;
use std::os::unix::io::OwnedFd;
use std::path::Path;

/// The flag policy for every device open performed through the seat.
pub fn open_flags() -> OFlags {
    OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK
}

pub fn open<S: Session>(session: &mut S, path: &Path) -> OwnedFd
where
    S::Error: std::fmt::Debug,
{
    session
        .open(path, open_flags())
        .unwrap_or_else(|e| abort!("session open of {path:?} failed: {e:?}"))
}

/// Best-effort open: `None` on failure instead of aborting. For enumerating OPTIONAL
/// devices (secondary GPU cards) where a device that can't be opened must be skipped,
/// not treated as fatal — unlike [`open`], which is for the required primary device.
pub fn try_open<S: Session>(session: &mut S, path: &Path) -> Option<OwnedFd>
where
    S::Error: std::fmt::Debug,
{
    session.open(path, open_flags()).ok()
}

pub fn close<S: Session>(session: &mut S, fd: OwnedFd)
where
    S::Error: std::fmt::Debug,
{
    // Close failure on teardown is the one tolerated log: the fd is gone
    // either way and the session may already be revoked (self-recovering).
    if let Err(e) = session.close(fd) {
        warn!("session close failed: {e:?}");
    }
}

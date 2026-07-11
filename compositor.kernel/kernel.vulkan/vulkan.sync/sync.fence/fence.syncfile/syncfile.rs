//! A smithay `Fence` backed by a `sync_file` fd (exported from a Vulkan binary
//! semaphore via `VK_KHR_external_semaphore_fd` / `SYNC_FD`). The async render
//! path returns this from `finish()` so the consumer waits on the real GPU
//! fence: winit imports it as an EGL native fence; native uses it as the
//! atomic-commit IN_FENCE.

use smithay::backend::renderer::sync::{Fence, Interrupted};
use std::os::unix::io::{AsRawFd, OwnedFd};

#[derive(Debug)]
pub struct SyncFileFence {
    fd: OwnedFd,
    /// `infence_2`: treat `POLLNVAL`/`POLLERR` as a failed wait (`Err`) instead of
    /// a fake success. The original `infence` path keeps the lenient behaviour
    /// (`strict = false`) to stay byte-identical.
    strict: bool,
}

impl SyncFileFence {
    pub fn new(fd: OwnedFd) -> Self {
        Self { fd, strict: false }
    }

    /// Strict variant (`infence_2`): a `POLLNVAL`/`POLLERR` on the fd is reported as
    /// `Interrupted` rather than a (fake) completed wait.
    pub fn new_strict(fd: OwnedFd) -> Self {
        Self { fd, strict: true }
    }

    /// poll(2) the sync_file: POLLIN means the fence is signaled.
    fn poll(&self, timeout_ms: i32) -> bool {
        let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        r > 0 && (pfd.revents & libc::POLLIN) != 0
    }
}

impl Fence for SyncFileFence {
    fn is_signaled(&self) -> bool {
        self.poll(0)
    }

    fn wait(&self) -> Result<(), Interrupted> {
        loop {
            let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let r = unsafe { libc::poll(&mut pfd, 1, -1) };
            if r > 0 {
                if pfd.revents & (libc::POLLNVAL | libc::POLLERR) != 0 {
                    warn!(
                        "SyncFileFence::wait: poll revents={:#x} on fd={} (invalid/err — NOT a real GPU wait)",
                        pfd.revents, self.fd.as_raw_fd()
                    );
                    // Strict (`infence_2`): a bad fd is a failed wait, not a fake
                    // completed one — the caller must not proceed as if synced.
                    if self.strict {
                        return Err(Interrupted);
                    }
                }
                return Ok(());
            }
            if r < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
            }
            return Err(Interrupted);
        }
    }

    fn is_exportable(&self) -> bool {
        true
    }

    fn export(&self) -> Option<OwnedFd> {
        self.fd.try_clone().ok()
    }
}

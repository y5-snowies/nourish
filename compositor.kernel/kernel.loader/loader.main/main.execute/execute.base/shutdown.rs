//! Ending the session on purpose: catching the signal, and putting back what only this
//! process was holding.
//!
//! Two halves, and they are separate because only one of them can run in signal context.
//! [`register`] turns SIGTERM and SIGINT into an ordinary loop wake-up that stops the
//! event loop, and everything that actually undoes something runs afterwards in
//! [`teardown`], on the main thread, as normal code.
//!
//! **The signal half is not optional, and its absence is what made logout leave a broken
//! console.** logind ends a session by sending SIGTERM. With no handler the process dies
//! with no unwinding at all, so no destructor runs — including the one in smithay's DRM
//! device that re-commits the mode the console was using. The monitor is then left being
//! driven by whatever the compositor programmed last, the getty never reprograms it, and
//! the display reports the timing as unsupported.
//!
//! It routes into `draw.state::lifecycle::stop`, the SAME function the Logout menu item
//! calls, so there is exactly one exit path to reason about rather than one per way of
//! being asked to leave.
//!
//! `signalfd` rather than a handler: a real handler could only safely poke a pipe anyway,
//! and the tree already hand-rolls this pattern for the launch reaper (see
//! `executor.install::install_reaper`, which explains that calloop's `signals` feature is
//! not enabled in the smithay reexport). Doing the same thing here adds no dependency and
//! keeps one idiom.
//!
//! SIGKILL is deliberately absent. It cannot be caught, so nothing here runs and the
//! console is left to the kernel. The defence is answering SIGTERM quickly enough that
//! systemd never escalates.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction};

use compositor_model_debug_instance_record::{info, warn};
use compositor_orchestration_core_state_base::Loop;
use compositor_orchestration_core_state_base::state::StatusSession;
use compositor_orchestration_environment_interface_lifecycle::lifecycle as env_lifecycle;
use compositor_support_smithay_state_xwayland_display::display;

/// Block SIGTERM and SIGINT process-wide, before any thread exists.
///
/// Call at the very top of `main()`. A `signalfd` only receives what is blocked, and the
/// mask is inherited by every thread spawned afterwards, so this has to precede the first
/// spawn — which in practice means the log threads, the first thing `main` starts.
///
/// The ONLY signal setup left in startup. The launch executor used to do the same thing
/// for SIGCHLD, because its reaper was a signalfd too; children are now collected through
/// their own exit descriptors, so that signal has no consumer and is left deliverable.
/// Its default action is to be ignored, and an ignored signal is discarded rather than
/// interrupting a blocking call, so leaving it unblocked costs the other threads nothing.
///
/// This does not leak into launched applications: `process.child::hygiene` resets the
/// mask to empty in the child before `exec`.
pub fn block_signals() {
    // SAFETY: standard signal-mask setup, and this is still the only thread.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

/// Watch for SIGTERM / SIGINT and stop the event loop when one arrives.
///
/// Failure is not fatal and must not be: a compositor that will not start because it
/// could not arrange its own shutdown is worse than one that exits untidily.
pub fn register(event_loop: &mut EventLoop<'static, Loop>) {
    // SAFETY: both signals are blocked by `block_signals`, so this fd is their sole
    // consumer and nothing else can steal a delivery.
    let sfd = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::signalfd(-1, &set, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC)
    };
    if sfd < 0 {
        warn!("signalfd(SIGTERM/SIGINT) failed; the session will not shut down gracefully");
        return;
    }
    // SAFETY: signalfd returned a fresh owned fd.
    let owned = unsafe { OwnedFd::from_raw_fd(sfd) };
    let inserted = event_loop.handle().insert_source(
        Generic::new(owned, Interest::READ, Mode::Level),
        move |_readiness, fd, state: &mut Loop| {
            // Drain the queued siginfo records (level-triggered) so the fd quiesces —
            // otherwise this re-fires forever while the loop winds down.
            let mut buf = [0u8; 128]; // size_of::<signalfd_siginfo>()
            loop {
                let n = unsafe {
                    libc::read(fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                };
                if n <= 0 {
                    break;
                }
            }
            info!("shutdown signal received; stopping the event loop");
            compositor_orchestration_draw_state_lifecycle::lifecycle::stop(state);
            Ok(PostAction::Continue)
        },
    );
    if let Err(err) = inserted {
        warn!("could not watch for shutdown signals: {err:?}");
    }
}

/// Undo what only this process was holding, after the event loop has stopped.
///
/// Two things, and they are gated differently.
///
/// The SLICE always goes. It is named after this process, so it can only ever hold this
/// compositor's own launches, and stopping it is safe on any backend.
///
/// The ENVIRONMENT is skipped when nested, mirroring `announce_session`: a development
/// session under the winit backend never published, so it has nothing to retract and
/// clearing would blank the HOST session's values.
///
/// `DISPLAY` is retracted here alongside `WAYLAND_DISPLAY` even though `xwayland::died`
/// also clears it, because the two answer different events. `died` is Xwayland crashing
/// while the session lives on, which is a real state — the X server can go without taking
/// the compositor with it. This is the session itself ending, where Xwayland is about to
/// follow us out and nothing has told anyone.
///
/// The display is deliberately NOT touched here — see the restore in `main`, which runs
/// first because it is the part the user is looking at.
pub fn teardown(state: &mut Loop, nested: bool) {
    // The apps first, and NOT gated on `nested`: the slice is keyed by our own pid, so it
    // can only ever name this compositor's own launches. A nested dev session stopping it
    // cannot touch the host's.
    match compositor_introspection_execution_launch_scope::scope::stop_session_slice() {
        Ok(()) => info!("session slice stopped; launched apps are going with us"),
        Err(err) => warn!("could not stop the session slice; apps may outlive us: {err}"),
    }
    if nested {
        return;
    }
    // Whether we are the session on screen, as the fallback rule only. Ownership is
    // normally decided by reading the values back; see `retract_session_env`.
    let active = matches!(state.inner.status_session, StatusSession::Active);
    let wayland_display = state.inner.loader.socket_name.to_string_lossy().into_owned();
    let x_display = display::get().unwrap_or_default();
    env_lifecycle::retract_session_env(
        active,
        &[("WAYLAND_DISPLAY", &wayland_display), ("DISPLAY", &x_display)],
    );
}

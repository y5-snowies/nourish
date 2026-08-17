use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use smithay::reexports::calloop::channel::{channel, Event};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};

use compositor_orchestration_core_state_base::Loop;
use compositor_introspection_execution_launch_policy::policy::{LaunchBackend, LaunchDispatch, LAUNCH_BACKEND, LAUNCH_DISPATCH};
use compositor_introspection_execution_launch_dispatch::dispatch::LaunchWorker;
use compositor_introspection_execution_launch_reap::reap::reap_zombies;
use compositor_introspection_execution_launch_types::types::LaunchOutcome;
use compositor_kernel_execution_driver_executor_base::executor::{BaseEnv, Executor, EXECUTOR};

/// Block SIGCHLD process-wide before any thread is spawned, so the reaper's
/// signalfd is the sole consumer. Call at the very top of `main()`. No-op under
/// the `Direct` backend.
pub fn block_sigchld() {
    if matches!(LAUNCH_BACKEND, LaunchBackend::Direct) {
        return;
    }
    // SAFETY: standard signal-mask setup; this is the only thread so far.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGCHLD);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

/// Build the Executor driver, store it in kernel storage, and wire its calloop
/// sources. After this the rim reads `EXECUTOR` to launch apps.
pub fn install(state: &mut Loop, handle: &LoopHandle<'static, Loop>) {
    let base_env = base_env(state);
    // Adopt launched PIDs into a systemd scope only when the SystemdScope backend
    // is active AND systemd is the init system. In a sandbox without systemd as
    // PID 1 (`systemctl` "has not been booted with systemd"), fall back to plain
    // self-spawn + reaper.
    let scope = matches!(LAUNCH_BACKEND, LaunchBackend::SystemdScope) && systemd_booted();

    let (tx, rx) = channel::<LaunchOutcome>();
    let worker = matches!(LAUNCH_DISPATCH, LaunchDispatch::OffThread)
        .then(|| LaunchWorker::spawn(tx.clone(), scope));
    // Register the driver slot (insert — the slot doesn't exist yet; get_mut would
    // panic on an unregistered slot, like the other driver slots in Orchestrator::new).
    state.inner.kernel.insert(&EXECUTOR, Some(Executor::new(worker, tx, base_env, scope)));

    // Outcome receiver → orchestration broadcasts the general Executed event.
    handle
        .insert_source(rx, |event, _, state: &mut Loop| {
            if let Event::Msg(outcome) = event {
                // Hand the focused world's router to the (core-independent) broadcast.
                compositor_orchestration_launch_broadcast_base::broadcast::broadcast(
                    state.inner.focus_channels(),
                    outcome,
                );
            }
        })
        .unwrap_or_else(|e| abort!("register launch outcome source: {e:?}"));

    install_reaper(handle);
}

/// systemd as the init system? Mirrors libsystemd's `sd_booted()`:
/// `/run/systemd/system` exists iff systemd is PID 1. When it isn't, `systemctl`
/// "has not been booted with systemd as init system" — so scope adoption is skipped.
fn systemd_booted() -> bool {
    std::path::Path::new("/run/systemd/system").exists()
}

/// Builds the faithful environment every launched app gets, RE-EVALUATED per
/// launch — see [`BaseEnv`] for why this is a closure and not a snapshot.
///
/// Only the socket name is captured, because it is fixed for the compositor's
/// lifetime (the listening socket is created once, before this runs). Anything
/// that could differ between launches is read inside the closure.
fn base_env(state: &Loop) -> BaseEnv {
    let wayland_display = state.inner.loader.socket_name.to_string_lossy().into_owned();
    Box::new(move || {
        let desktop = compositor_orchestration_environment_type_base::base::Get().DesktopName;
        vec![
            ("WAYLAND_DISPLAY".into(), wayland_display.clone()),
            // Deliberately EMPTY: keeps apps off the X11 fallback. A real
            // display here would make Electron/SDL/Java and some Qt configs
            // PREFER X11 over Wayland, so the satellite's `:12` belongs only on
            // the launches that are actually X11 apps — not on every child.
            ("DISPLAY".into(), String::new()),
            ("XDG_SESSION_TYPE".into(), "wayland".into()),
            ("XDG_CURRENT_DESKTOP".into(), desktop.clone()),
            ("XDG_SESSION_DESKTOP".into(), desktop),
        ]
    })
}

/// signalfd(SIGCHLD) wrapped in a Generic source — calloop's `signals` feature
/// isn't enabled in the smithay reexport. Gated on a reaped backend.
fn install_reaper(handle: &LoopHandle<'static, Loop>) {
    if matches!(LAUNCH_BACKEND, LaunchBackend::Direct) {
        return;
    }
    // SAFETY: SIGCHLD already blocked (block_sigchld); signalfd is sole consumer.
    let sfd = unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGCHLD);
        libc::signalfd(-1, &set, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC)
    };
    if sfd < 0 {
        warn!("signalfd(SIGCHLD) failed; launch reaper disabled");
        return;
    }
    // SAFETY: signalfd returned a fresh owned fd.
    let owned = unsafe { OwnedFd::from_raw_fd(sfd) };
    handle
        .insert_source(Generic::new(owned, Interest::READ, Mode::Level), |_readiness, fd, _state: &mut Loop| {
            // Drain queued siginfo (level-triggered) so the fd quiesces.
            let mut buf = [0u8; 128]; // size_of::<signalfd_siginfo>()
            loop {
                let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                if n <= 0 {
                    break;
                }
            }
            let _ = reap_zombies();
            Ok(PostAction::Continue)
        })
        .unwrap_or_else(|e| abort!("register SIGCHLD reaper: {e:?}"));
}

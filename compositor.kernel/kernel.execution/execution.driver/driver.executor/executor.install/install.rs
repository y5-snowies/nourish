use std::os::fd::OwnedFd;

use smithay::reexports::calloop::channel::{channel, Channel, Event};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};

use compositor_orchestration_core_state_base::Loop;
use compositor_introspection_execution_launch_policy::policy::{LaunchBackend, LaunchDispatch, LAUNCH_BACKEND, LAUNCH_DISPATCH};
use compositor_introspection_execution_launch_dispatch::dispatch::LaunchWorker;
use compositor_introspection_execution_launch_types::types::LaunchOutcome;
use compositor_kernel_execution_driver_executor_base::executor::{BaseEnv, Executor, EXECUTOR};
use compositor_support_library_process_child_pidfd::pidfd;

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
    // Where EVERY detached spawn in the process sends its child's exit descriptor, not
    // just launches: the kill helpers, the input method and the debug terminal all reach
    // it through `spawn_detached`. Installed once, here, because this is where the loop
    // that reaps is already being wired.
    let (watch_tx, watch_rx) = channel::<OwnedFd>();
    pidfd::install(move |watch| {
        let _ = watch_tx.send(watch);
    });
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

    install_reaper(handle, watch_rx);
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
            // The X display, once y5's own XWayland server is up — read live, which is
            // the whole reason this is a closure: the server picks its display
            // asynchronously, long after the executor was built.
            //
            // Empty until then, and empty forever if XWayland never starts. An empty
            // `DISPLAY` is what keeps apps off the X11 fallback; a real one makes some
            // Electron/SDL/Java and Qt configurations PREFER X11 over Wayland. That
            // cost is accepted now that the X server is ours: an X11-only app cannot
            // run at all without it, and every other compositor exports it. Apps that
            // must be forced back onto Wayland can say so in their own desktop entry
            // (`Exec=env GDK_BACKEND=wayland …`), as they already do for the toolkit
            // hints y5 deliberately does not set.
            (
                "DISPLAY".into(),
                compositor_support_smithay_state_xwayland_display::display::get().unwrap_or_default(),
            ),
            ("XDG_SESSION_TYPE".into(), "wayland".into()),
            ("XDG_CURRENT_DESKTOP".into(), desktop.clone()),
            ("XDG_SESSION_DESKTOP".into(), desktop),
        ]
    })
}

/// Reap each detached child when its exit descriptor reports readable.
///
/// `watches` carries one descriptor per detached spawn anywhere in the process; this
/// registers each as its own source and removes it once the child is collected, so the
/// set of watched processes is exactly the set still running.
///
/// It replaced a SIGCHLD `signalfd` that answered by waiting on ANY child, which is why
/// there used to be a mutex around every spawn in the process, a retry loop behind it,
/// and a worker thread to absorb the wait. Waiting on a child by NAME collides with
/// nothing — not with `Command::spawn`'s own wait after a failed exec, which asserts and
/// would panic the spawning thread; not with the `status` / `output` wrappers, which
/// would otherwise report a subprocess failure that never happened; and not with
/// smithay's Xwayland reaper. See `child.pidfd`.
///
/// Reaping runs HERE, on the loop, rather than on the launch worker. The wait is
/// non-blocking and the descriptor only wakes once the process has already exited, so
/// there is nothing left to stall on — which was the whole reason it lived off-thread.
fn install_reaper(handle: &LoopHandle<'static, Loop>, watches: Channel<OwnedFd>) {
    handle
        .insert_source(watches, |event, _, state: &mut Loop| {
            let Event::Msg(watch) = event else { return };
            // Registered from inside a callback, which calloop allows. The handle comes
            // off the state rather than being captured, so this closure does not pin the
            // loop it is registered on.
            let inserted = state.loop_handle.insert_source(
                Generic::new(watch, Interest::READ, Mode::Level),
                |_readiness, watch, _state: &mut Loop| {
                    // One shot: the process has exited, so once collected there is
                    // nothing further this descriptor can report.
                    pidfd::reap(watch);
                    Ok(PostAction::Remove)
                },
            );
            if let Err(e) = inserted {
                warn!("could not watch a launched child for exit: {e:?}");
            }
        })
        .unwrap_or_else(|e| abort!("register launch reaper: {e:?}"));
}

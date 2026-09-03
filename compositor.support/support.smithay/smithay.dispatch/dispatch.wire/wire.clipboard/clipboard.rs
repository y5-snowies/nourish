//! Event-loop side of clipboard persistence.
//!
//! `Dispatch` records what it wants (see the `SelectionHandler` impl in
//! `dispatch.state/state.base`) but owns no `loop_handle`, so what needs one is set up
//! here: starting the transfer worker, handing it the pipes for each direction, and
//! admitting the flavors it finishes.
//!
//! No byte of a transfer moves on this thread. Reads and writes both used to run here on
//! calloop sources, and that is what made a large copy or paste a stall — see
//! `clipboard.worker`. What is left is fd handover and bookkeeping.
//!
//! Two things this deliberately does NOT do:
//!
//! - It does not detect that the clipboard died. smithay asks us for a replacement from
//!   `selection_source_destroyed` instead, so the selection is swapped in one transition
//!   rather than cleared and refilled.
//! - It does not capture inside `new_selection`. That callback fires *before* smithay
//!   installs the new selection, so reading there would read the previous clipboard.
//!
//! Nothing here may log clipboard CONTENT — mime names, byte counts and generations only.

use compositor_support_smithay_dispatch_state_base::state::Dispatch;
use compositor_support_smithay_state_clipboard_policy::policy;
use compositor_support_smithay_state_clipboard_worker::worker::Worker;
use smithay::reexports::calloop::ping::make_ping;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::rustix;
use compositor_support_smithay_dispatch_state_base::state::xwm_impls::X11_SELECTION;
use smithay::wayland::selection::data_device::{
    current_data_device_selection_userdata, request_data_device_client_selection,
};

/// Run one pass: take what the worker finished, hand it the next copy, serve queued pastes.
///
/// `project` reaches the protocol state from whatever type the event loop is generic
/// over — the rim passes `|wire| &mut wire.state`.
pub fn drain<W: 'static>(
    state: &mut Dispatch,
    handle: &LoopHandle<'static, W>,
    project: fn(&mut W) -> &mut Dispatch,
) {
    collect(state);
    arm(state, handle, project);
    serve(state);
}

/// Admit every flavor the worker has finished since the last pass.
///
/// Also runs from the worker's ping, so a completion is picked up as soon as it happens
/// rather than waiting for the next client request to drive a drain.
pub fn collect(state: &mut Dispatch) {
    let Some(worker) = state.clipboard.worker.as_ref() else {
        return;
    };
    for done in worker.collect() {
        let len = done.bytes.len();
        let kept = state.clipboard.capture.admit(
            done.generation,
            done.order,
            done.mime.clone(),
            done.bytes,
            policy::BUDGET,
        );
        trace!("clipboard captured mime={} bytes={len} kept={kept}", done.mime);
    }
}

/// Start the capture worker, once, on the first copy.
///
/// Deferred to here rather than the factory because the worker needs a calloop `Ping` to
/// wake the compositor with, and only this side has a loop handle. A failure is not fatal:
/// the clipboard simply stops persisting across client exit.
fn start_worker<W: 'static>(
    state: &mut Dispatch,
    handle: &LoopHandle<'static, W>,
    project: fn(&mut W) -> &mut Dispatch,
) {
    if state.clipboard.worker.is_some() {
        return;
    }
    let Ok((ping, source)) = make_ping() else {
        warn!("clipboard capture: could not create the worker ping");
        return;
    };
    let inserted = handle.insert_source(source, move |_, _, outer| collect(project(outer)));
    if inserted.is_err() {
        warn!("clipboard capture: could not register the worker ping");
        return;
    }
    state.clipboard.worker = Worker::start(ping);
}

/// Hand the worker every flavor of the selection that was just set.
///
/// Two owners, one path. A wayland client is asked through
/// `request_data_device_client_selection`; an X11 owner through
/// `X11Wm::send_selection`, which converts the X selection and writes it to the fd
/// asynchronously over the X connection. Everything either side of that one call —
/// budget, generation, pipe widening, the worker — is deliberately identical, because
/// the whole point is that a clipboard copied in an X app survives that app exiting
/// exactly as one copied in a wayland app does.
///
/// Which owner is asked is read off the SEAT rather than recorded alongside it, the same
/// discriminator `XwmHandler::cleared_selection` uses: the selection's user data already
/// says who owns the clipboard, exactly once, and a second copy of that fact could only
/// ever agree with it or be a silent bug.
fn arm<W: 'static>(
    state: &mut Dispatch,
    handle: &LoopHandle<'static, W>,
    project: fn(&mut W) -> &mut Dispatch,
) {
    let Some((generation, mime_types)) = state.clipboard.pending_capture.take() else {
        return;
    };
    if !state.clipboard.capture.is_current(generation) {
        return;
    }
    start_worker(state, handle, project);
    if state.clipboard.worker.is_none() {
        return;
    }
    let x11_owned = {
        let held = current_data_device_selection_userdata::<Dispatch>(&state.seat.seat);
        held.is_some_and(|user_data| *user_data == X11_SELECTION)
    };
    let seat = state.seat.seat.clone();
    // The requests are dispatched first and handed to the worker after, because asking
    // the X owner needs `&mut state` (the `X11Wm` lives there) while the worker is
    // reached through a shared borrow of the same state.
    let mut handed: Vec<(usize, String, std::os::fd::OwnedFd, usize)> = Vec::new();
    for (order, mime) in mime_types {
        let pipe = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC);
        let Ok((read, write)) = pipe else {
            warn!("clipboard capture: pipe failed mime={mime}");
            continue;
        };
        // ONLY our end is non-blocking. The client's end stays blocking on purpose: it
        // is what makes "the client exited cleanly" imply "the client finished writing"
        // (anything past the pipe buffer blocks it until we consume), and naive clients
        // do not handle EAGAIN. (An X11 owner is not held to that: smithay sets the fd
        // it is handed non-blocking itself, and the X side is a conversion it drives
        // rather than a client writing at its own pace.)
        let nonblock = rustix::fs::fcntl_setfl(
            &read,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK,
        );
        if let Err(err) = nonblock {
            warn!("clipboard capture: O_NONBLOCK failed mime={mime} err={err:?}");
            continue;
        }
        let chunk = widen(&read);
        // Either call hands `write` away and drops it, leaving the owner as the pipe's
        // only writer — without that it would never reach EOF.
        let requested = if x11_owned {
            match state.xwayland.xwm.as_mut() {
                Some(xwm) => xwm
                    .send_selection(smithay::wayland::selection::SelectionTarget::Clipboard, mime.clone(), write)
                    .map_err(|err| format!("{err:?}")),
                None => Err("no x11 window manager".to_string()),
            }
        } else {
            request_data_device_client_selection::<Dispatch>(&seat, mime.clone(), write)
                .map_err(|err| format!("{err:?}"))
        };
        if let Err(err) = requested {
            trace!("clipboard capture: selection unavailable mime={mime} err={err}");
            continue;
        }
        handed.push((order, mime, read, chunk));
    }
    let Some(worker) = state.clipboard.worker.as_ref() else {
        return;
    };
    // Re-armed here even though `new_selection` already did it: on the very first copy the
    // worker did not exist yet when that hook ran, and a `Read` for a generation the worker
    // has not been armed with is dropped. Arming is idempotent.
    worker.arm(generation);
    let armed = handed.len();
    for (order, mime, read, chunk) in handed {
        worker.read(generation, order, mime, read, chunk);
    }
    trace!("clipboard capture armed generation={generation} flavors={armed} x11={x11_owned}");
}

/// Widen OUR capture pipe as far as this machine allows, and report the capacity one read
/// should aim for.
///
/// `F_SETPIPE_SZ` is denied above `/proc/sys/fs/pipe-max-size` — and a denied request leaves
/// the pipe at its DEFAULT capacity, so overshooting costs rather than clamps. It can also
/// be refused well below that limit: the per-user pipe page budget counts toward it, and
/// some sandboxes block the call outright. None of that is knowable up front and all of it
/// varies per machine, so this asks for the most and halves until the kernel agrees.
///
/// The capacity is then read back rather than assumed. It is what a read should attempt:
/// more than the pipe can hold is dead weight on every syscall, less is extra syscalls.
fn widen(fd: &std::os::fd::OwnedFd) -> usize {
    let mut want = policy::CAPTURE_PIPE;
    while want > policy::READ_CHUNK {
        if let Ok(capacity) = rustix::pipe::fcntl_setpipe_size(fd, want) {
            return capacity;
        }
        want /= 2;
    }
    rustix::pipe::fcntl_getpipe_size(fd).unwrap_or(policy::READ_CHUNK)
}

/// Hand a paste to the worker.
///
/// The write itself is not done here. A payload past the pipe buffer would otherwise be
/// pushed inline on the compositor thread — a client that drains as fast as we fill keeps
/// the loop going for the whole payload — and no timer can interrupt a running callback,
/// so the deadline has to be enforced by whoever owns the loop. That is the worker.
fn serve(state: &mut Dispatch) {
    for (generation, client, mime, fd) in std::mem::take(&mut state.clipboard.pending_sends) {
        if !state.clipboard.capture.is_current(generation) {
            continue;
        }
        let Some(bytes) = state.clipboard.capture.bytes_for(&mime) else {
            trace!("clipboard read for a flavor we do not hold mime={mime}");
            continue;
        };
        let nonblock = rustix::fs::fcntl_setfl(
            &fd,
            rustix::fs::OFlags::WRONLY | rustix::fs::OFlags::NONBLOCK,
        );
        if let Err(err) = nonblock {
            warn!("clipboard read: O_NONBLOCK failed mime={mime} err={err:?}");
            continue;
        }
        // Only reachable with a worker: a paste of a persisted flavor presupposes a capture,
        // and a capture is what starts the worker. Dropping `fd` here would close the pipe,
        // which the client reads as an empty transfer.
        let Some(worker) = state.clipboard.worker.as_ref() else {
            warn!("clipboard read: no worker to serve mime={mime}");
            continue;
        };
        worker.write(client, mime, fd, bytes);
    }
}

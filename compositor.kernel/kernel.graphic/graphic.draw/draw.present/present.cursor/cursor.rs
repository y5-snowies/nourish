//! Frame callbacks for the surfaces the COMPOSITOR draws itself.
//!
//! Separate from `present.callbacks` because the question is different. That crate sends
//! frames to WINDOWS and LAYER SURFACES — things a client placed and the compositor
//! arranges. These are things the compositor draws on top of everything: the cursor and
//! the drag-and-drop icon. They are in no `Space` and no layer map, so nothing there
//! reaches them.

use smithay::desktop::utils::send_frames_surface_tree;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use std::time::Duration;
use compositor_orchestration_core_state_base::Loop;

/// Send frame callbacks to the cursor and the drag icon.
///
/// That is not cosmetic. A client paces its own updates on frame callbacks: it attaches a
/// buffer, commits, and waits to be told the frame was shown before preparing the next
/// one. A surface that is never told simply stops — so the CURSOR TEXTURE FREEZES at
/// whichever image happened to be current, and does not come back, because the client is
/// not withholding the revert so much as still waiting to be released for it. It reads as
/// "the cursor sometimes does not update, and once it is wrong it stays wrong" — the
/// I-beam that survives leaving the text.
///
/// XWayland shows it first because Xwayland keeps ONE cursor surface per seat and swaps
/// buffers on it for every X cursor, so every cursor change after the first goes through
/// this path. It is not an XWayland bug: any wayland client that paces its cursor the same
/// way is affected, which is why it was visible through xwayland-satellite too — satellite
/// is just another client whose cursor surface lands in the same gap.
///
/// `Some(Duration::ZERO)` throttle, matching the window path: fire on every frame this
/// output presents rather than rate-limiting a surface the user is looking straight at.
///
/// Scoped to the output the cursor is actually ON — the third of three answers to the
/// same question, and the only one that had none. Windows are narrowed upstream, by the
/// per-output scene build (`vis.drawn`: this camera's frustum, minus what is occluded);
/// layers by smithay's per-output layer map. This surface is in neither, so without the
/// guard it was the one thing paced by EVERY output's present — the sum of their refresh
/// rates, an animated cursor running at 204Hz on a 144+60 desktop.
///
/// A window on two outputs is still fired by both, and should be: it is in both drawn
/// sets, so it is paced by the faster of the monitors actually showing it. There is only
/// ever one cursor and one drag icon, on one monitor, so there is no equivalent claim.
///
/// Not a throttle. Rate-limiting the one surface the eye is tracking is the wrong
/// instrument, and the extra sends were nearly free anyway (callbacks are consumed by the
/// first sender, leaving the rest a no-op tree walk). What this fixes is which output's
/// cadence the client is paced by.
pub fn send_frames(state: &Loop, output: &Output) {
    if compositor_orchestration_core_state_base::state::output_key(output)
        != state.inner.active_output_key()
    {
        return;
    }
    let frame_time = state.inner.start_time.elapsed();
    let mut send = |surface: &WlSurface| {
        send_frames_surface_tree(surface, output, frame_time, Some(Duration::ZERO), |_, _| {
            Some(output.clone())
        });
    };
    if let smithay::input::pointer::CursorImageStatus::Surface(surface) =
        &state.state.seat.pointer_status
    {
        send(surface);
    }
    // The drag icon is the same shape of surface with the same gap: compositor-drawn,
    // in no Space, and equally able to stall waiting for a callback.
    if let Some(icon) = &state.state.dnd.icon {
        send(icon);
    }
}


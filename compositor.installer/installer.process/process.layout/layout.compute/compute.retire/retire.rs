//! Retiring what a PREVIOUS install left behind.
//!
//! Separate from `compute.units` because it is the opposite operation: those
//! functions place things we own, this one removes something we used to. The
//! distinction matters here more than it usually would — the artefact is a systemd
//! unit under a generic name, so removing it safely is a question of PROVING it is
//! ours, not of knowing where we put it.

use std::path::PathBuf;
use compositor_installer_process_layout_compute_stage::{Action, user_systemd_dir};

/// Retire the xwayland-satellite deployment — by DISABLING its service, nothing more.
///
/// X11 support installs nothing now: the compositor runs Xwayland itself (it execs
/// the `Xwayland` binary and is that server's X11 window manager in-process), so
/// there is no proxy binary and no user service. The one requirement is the X server
/// package, which the default-on `xwayland` package group installs.
///
/// But a machine installed before this still has the satellite enabled, and leaving
/// it running would have it racing the compositor's own server for a display number
/// — so every install retires it.
///
/// Guarded by [`is_our_unit`] rather than by the name: `xwayland.service` is a name
/// anyone could have used, and disabling and deleting a unit we did not write is not
/// ours to do. (The name was a poor choice on our part; the marker is how we live
/// with it.) Nothing is emitted at all when there is no unit or it is not ours,
/// which is the case on every fresh install.
pub fn xwayland_retire_actions() -> Vec<Action> {
    let unit = user_systemd_dir().join("xwayland.service");
    if !is_our_unit(&unit) {
        return vec![];
    }
    // DISABLE ONLY — nothing is deleted, not the unit and not
    // `/usr/bin/xwayland-satellite`.
    //
    // Stopping the service is the whole requirement: what a surviving satellite does is
    // race the compositor's own server for a display number, and a disabled unit races
    // nothing. Deleting is a different act with a different risk profile — the binary
    // may be the distro's or the user's, and even the unit is a file we would be
    // removing on the strength of a string match. An inert file costs nothing; an
    // unwanted deletion cannot be undone.
    //
    // No `daemon-reload` either: nothing on disk changed, and the plan issues one at
    // the end regardless.
    //
    // Consequence worth knowing: the unit file stays, so this re-emits a `disable` on
    // every later install. `systemctl disable --now` on an already-disabled unit
    // succeeds and does nothing, which is the right trade for not deleting.
    vec![Action::SystemctlUser(vec![
        "disable".into(),
        "--now".into(),
        "xwayland.service".into(),
    ])]
}

/// Is the unit at `path` the one y5 shipped?
///
/// ONE marker, and it has to name y5: the `Description` line from the unit we used to
/// install. That unit is no longer in the tree — the whole satellite component is gone —
/// so the string below is now the ONLY record of what it said. Do not "tidy" it.
///
/// An earlier version also accepted any `ExecStart=` mentioning `xwayland-satellite`,
/// which was wrong — that identifies the PROGRAM, not us. Anyone running upstream
/// xwayland-satellite from their own unit at this perfectly ordinary path would have
/// had it disabled and deleted by a y5 install.
///
/// The two failure directions are not symmetric, which is what settles the choice. A
/// false negative leaves a satellite we did install still running: the user sees a
/// display-number race, and can fix it. A false positive deletes a unit someone else
/// wrote, silently and unrecoverably. So the test errs toward doing nothing, and a
/// hand-edited `Description` simply means y5 leaves the unit alone.
///
/// An unreadable file is not ours either — the whole point is to touch nothing we
/// cannot identify.
///
/// The uninstall script matches on the same marker; keep the two in step.
pub fn is_our_unit(path: &PathBuf) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    text.lines()
        .any(|line| line.trim().starts_with("Description=X Wayland Satellite (y5"))
}

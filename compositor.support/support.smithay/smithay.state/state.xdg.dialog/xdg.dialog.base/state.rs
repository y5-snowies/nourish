//! `xdg_wm_dialog_v1` — the only way a client can say "this toplevel is a
//! dialog", and specifically a *modal* one.
//!
//! Core xdg-shell has `xdg_toplevel.set_parent` ("transient for") and stops
//! there; X11 had both that and `_NET_WM_STATE_MODAL`. This protocol restores
//! the missing half. It is a marker only — the relation to the parent still
//! comes from `set_parent`, and the hint says nothing about which window the
//! dialog belongs to.
//!
//! Holding the global alive is the whole job of this type. The hint itself is
//! stored by smithay in `XdgToplevelSurfaceData.dialog_hint` and delivered to
//! the handler as `ToplevelDialogHint`.

use smithay::wayland::shell::xdg::dialog::XdgDialogState;

pub struct Dialog {
    pub xdg_dialog_state: XdgDialogState,
}

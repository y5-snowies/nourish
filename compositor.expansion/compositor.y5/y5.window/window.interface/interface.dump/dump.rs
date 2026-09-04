//! On-demand dump: what is on screen, then the focused window's meta tree.
//!
//! Shortcut-bound rather than run at map time because the walk reads `/proc` for
//! the window's process AND every ancestor — far too heavy for a lifecycle path,
//! and only ever wanted while looking at one specific window.
//!
//! The ancestor chain is the point: what identifies the real application (a Steam
//! launcher, a Proton wrapper) often sits ABOVE the window's own process node.
//! Printing the chain is the only way to see that shape on a live window. Native
//! XWayland keeps this honest for X11 windows too — the pid comes from
//! `_NET_WM_PID`, the app's own, not from the surface credentials that would name
//! the one X server for every X11 window on screen.

use compositor_introspection_extraction_window_meta_types::types::MetaNode;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_record::window::LoopWindow;
use smithay::desktop::Window;
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
use std::collections::HashSet;
use compositor_support_smithay_state_window_find::find;
use compositor_support_smithay_state_window_ident::ident;

/// Emitted at `error!` so it lands whatever the configured level: this only ever
/// runs when a human pressed the key, and a diagnostic that the log level can
/// swallow is a diagnostic that wastes the press.
fn line(depth: usize, node: &MetaNode) {
    let m = &node.meta;
    error!(
        "meta[{depth}] app_id={:?} title={:?} pid={:?} comm={:?} exe={:?} cwd={:?} cgroup={:?} cmdline={:?} env={:?}",
        m.app_id, m.title, m.pid, m.comm, m.exe, m.cwd, m.cgroup, m.cmdline, m.selected_env
    );
}

/// The focused toplevel, or `None` when focus is on a layer surface, a popup, or
/// nothing at all.
fn focused(state: &Loop) -> Option<Window> {
    let surface = state.state.seat.seat.get_keyboard()?.current_focus()?;
    state.inner.space_state().state.elements()
        .find(|w| find::is_surface(w, &surface))
        .cloned()
}

/// `title`, falling back to `[app_id]`: a blank title is common and names nothing.
fn label(window: &Window) -> String {
    let names = ident::names(window);
    if let Some(title) = names.title {
        return title;
    }
    match names.app_id {
        Some(app_id) => format!("[{app_id}]"),
        None => "<unnamed>".into(),
    }
}

/// Everything on a pane this frame, deduped across slots, in stacking order.
///
/// `Viewports::on_pane` is exactly that, not the drawn set: the scene also
/// drops windows fully covered by opaque ones in front, and the "full" fractional
/// strategy adds the grace band just off the pane. Right for "what is around
/// here", wrong as "these got frame callbacks".
fn on_pane(state: &Loop) -> Vec<String> {
    let ids: HashSet<uuid::Uuid> = state.inner.viewports().on_pane_grace.values().flatten().copied().collect();
    state.inner.space_state().state.elements()
        .filter(|w| w.uuid().is_some_and(|u| ids.contains(&u))).map(label).collect()
}

/// Dump the focused window's meta tree: the window's own process first, then
/// each ancestor outward, then the expanded children.
pub fn focused_meta(state: &mut Loop) {
    // First and unconditional: everything below needs a focused window, so with
    // nothing selected the whole press used to produce one line saying so.
    let on = on_pane(state);
    error!("meta dump: on-pane windows ({}): {}", on.len(), on.join(" | "));
    let Some(window) = focused(state) else {
        error!("meta dump: nothing focused");
        return;
    };
    let space = &state.inner.space_state().state;
    let dh = &state.inner.loader.display_handle;
    let Some(node) = window.meta(space, dh) else {
        error!("meta dump: no metadata for the focused window");
        return;
    };
    error!("meta dump: focused window ─────────────────────────────");
    let mut at = Some(&node);
    let mut depth = 0;
    while let Some(n) = at {
        line(depth, n);
        at = n.parent.as_deref();
        depth += 1;
    }
    for (i, child) in node.children.iter().enumerate() {
        error!("meta dump: child {i}");
        line(0, child);
    }
}

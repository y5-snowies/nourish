//! On-demand dump: what is on screen, then the focused window's meta tree.
//!
//! Shortcut-bound rather than run at map time because the walk reads `/proc` for
//! the window's process AND every ancestor — far too heavy for a lifecycle path,
//! and only ever wanted while looking at one specific window.
//!
//! The ancestor chain is the point: under xwayland every X11 title shares ONE
//! satellite process, so the window's own node describes the satellite, and what
//! identifies the real application (a Steam launcher, a Proton wrapper) sits above
//! it. Printing the chain is the only way to see that shape on a live window.

use compositor_introspection_extraction_window_meta_types::types::MetaNode;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_record::window::LoopWindow;
use smithay::desktop::Window;
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
use std::collections::HashSet;

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
        .find(|w| w.toplevel().map(|t| t.wl_surface() == &surface).unwrap_or(false))
        .cloned()
}

/// `title`, falling back to `[app_id]`: a blank title is common and names nothing.
fn label(window: &Window) -> String {
    let Some(s) = window.toplevel().map(|t| t.wl_surface().clone()) else { return "<no toplevel>".into() };
    with_states(&s, |st| {
        let Some(a) = st.data_map.get::<XdgToplevelSurfaceData>().and_then(|a| a.lock().ok()) else {
            return "<no role data>".into();
        };
        let t = a.title.clone().unwrap_or_default();
        if t.is_empty() { format!("[{}]", a.app_id.clone().unwrap_or_default()) } else { t }
    })
}

/// Everything on a pane this frame, deduped across slots, in stacking order.
///
/// `Viewports::visible` is the ON-PANE set, not the drawn set: the scene also
/// drops windows fully covered by opaque ones in front, and the "full" fractional
/// strategy adds the grace band just off the pane. Right for "what is around
/// here", wrong as "these got frame callbacks".
fn on_pane(state: &Loop) -> Vec<String> {
    let ids: HashSet<uuid::Uuid> = state.inner.viewports().visible.values().flatten().copied().collect();
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

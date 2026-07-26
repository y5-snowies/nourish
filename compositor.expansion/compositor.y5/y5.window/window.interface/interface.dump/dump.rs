//! On-demand dump of the focused window's introspection meta tree.
//!
//! Bound to a shortcut rather than run at map time on purpose: the walk reads
//! `/proc` for the window's process AND every ancestor, which is far too heavy to
//! do on a lifecycle path, and the answer is only ever wanted while looking at a
//! specific window.
//!
//! The ancestor chain is the point. Under xwayland every X11 title shares ONE
//! satellite process, so the window's own node describes the satellite, and
//! whatever identifies the real application — a Steam launcher, a Proton wrapper
//! — is somewhere above it. Printing the chain is the only way to see that shape
//! on a live window instead of guessing at it.

use compositor_introspection_extraction_window_meta_types::types::MetaNode;
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_window_interface_record::window::LoopWindow;
use smithay::desktop::Window;

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
    state
        .inner
        .space_state()
        .state
        .elements()
        .find(|w| w.toplevel().map(|t| t.wl_surface() == &surface).unwrap_or(false))
        .cloned()
}

/// Dump the focused window's meta tree: the window's own process first, then
/// each ancestor outward, then the expanded children.
pub fn focused_meta(state: &mut Loop) {
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

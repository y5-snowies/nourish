use compositor_introspection_extraction_window_meta_proc_read::proc::extract_meta_for_pid;
use compositor_introspection_extraction_window_meta_proc_tree::proc::extract_full_tree;
use compositor_introspection_extraction_window_meta_types::types::{Meta, MetaNode};
use compositor_introspection_extraction_window_meta_wayland_surface::wayland::read_surface_identity;
use smithay::desktop::{Space, Window};
use smithay::reexports::wayland_server::DisplayHandle;
use compositor_support_smithay_state_window_ident::ident;

/// Pull a `Meta` from a live Wayland window, joining its surface identity
/// with the /proc data for its client process. `Some` if at least one data
/// source (Wayland or /proc) yielded something.
pub fn extract_from_window(
    window: &Window,
    _space: &Space<Window>,
    display_handle: &DisplayHandle,
) -> Option<Meta> {
    let (app_id, title, _target_wl_surface) = read_surface_identity(window);
    let creds = ident::credentials(window, display_handle);

    let mut meta = if let Some(p) = creds.pid {
        extract_meta_for_pid(p).unwrap_or_default()
    } else {
        Meta::default()
    };

    meta.app_id = app_id;
    meta.title = title;
    meta.pid = creds.pid;
    meta.uid = creds.uid;
    meta.gid = creds.gid;

    if meta.app_id.is_none() && meta.title.is_none() && meta.pid.is_none() && meta.exe.is_none() {
        return None;
    }

    Some(meta)
}

/// The WAYLAND half of [`extract_from_window`] alone — surface identity plus
/// client credentials, with no `/proc` walk at all.
///
/// This is what the compositor's main thread can afford to run over every
/// window at once (the overview does, on open): the fields it fills are exactly
/// the ones the sampler thread cannot refresh for itself, and the expensive
/// process-tree half stays on that thread. `None` when the window yields
/// nothing identifying.
pub fn extract_surface_meta(window: &Window, display_handle: &DisplayHandle) -> Option<Meta> {
    let (app_id, title, _target_wl_surface) = read_surface_identity(window);
    let mut meta = Meta { app_id, title, ..Meta::default() };

    let creds = ident::credentials(window, display_handle);
    meta.pid = creds.pid;
    meta.uid = creds.uid;
    meta.gid = creds.gid;

    if meta.app_id.is_none() && meta.title.is_none() && meta.pid.is_none() {
        return None;
    }
    Some(meta)
}

/// Same as `extract_from_window`, but also expands the process tree
/// (children + parents) into a `MetaNode`.
pub fn extract_node_from_window(
    window: &Window,
    space: &Space<Window>,
    display_handle: &DisplayHandle,
) -> Option<MetaNode> {
    let root_meta = extract_from_window(window, space, display_handle)?;

    // If we got a PID, build the full tree and replace its root meta with
    // the Wayland-enriched version (so app_id/title/uid/gid are preserved).
    if let Some(pid) = root_meta.pid {
        let mut node = extract_full_tree(pid).unwrap_or(MetaNode::leaf(root_meta.clone()));
        node.meta.app_id = root_meta.app_id;
        node.meta.title = root_meta.title;
        node.meta.uid = root_meta.uid;
        node.meta.gid = root_meta.gid;
        Some(node)
    } else {
        Some(MetaNode::leaf(root_meta))
    }
}

/// Extract a `MetaNode` from a live Wayland window. **Must be called while
/// the window and its process are alive.** `None` if the window has no
/// surface, no process credentials, or `/proc/<pid>` isn't readable.
pub fn extract_meta(
    window: &Window,
    space: &Space<Window>,
    display_handle: &DisplayHandle,
) -> Option<MetaNode> {
    extract_node_from_window(window, space, display_handle)
}

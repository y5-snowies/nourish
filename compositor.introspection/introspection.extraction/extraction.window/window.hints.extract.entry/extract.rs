use compositor_introspection_extraction_window_desktop_search::desktop::find_by_app_id;
use compositor_introspection_extraction_window_hints_attributes_identity::attributes::{
    DBusActivatable, DesktopEntryPath, DisplayName, IconName, IconPath, XdgIconName,
};
use compositor_introspection_extraction_window_hints_attributes_identity_more::attributes::{AppId, Title};
use compositor_introspection_extraction_window_hints_attributes_launch::attributes::EnvOverlay;
use compositor_introspection_extraction_window_hints_inferred::inferred::InferredHints;
use compositor_introspection_extraction_window_hints_source::source::{Confidence, SourceMethod};
use compositor_introspection_extraction_window_hints_values::values::{EnvPair, ToplevelIcon};
use compositor_introspection_extraction_window_icon::icon::resolve as resolve_icon;
use compositor_introspection_extraction_window_meta_types::types::Meta;
use std::path::PathBuf;

/// Environment-derived hints: the allowlisted env overlay plus the
/// GIO_LAUNCHED_DESKTOP_FILE identity signal.
pub fn push_env_hints(meta: &Meta, hints: &mut InferredHints) {
    let Some(env) = &meta.selected_env else { return };
    if !env.is_empty() {
        let pairs: Vec<EnvPair> = env
            .iter()
            .map(|(k, v)| EnvPair { key: k.clone(), value: v.clone() })
            .collect();
        hints.push::<EnvOverlay>(
            pairs,
            SourceMethod::ProcEnviron,
            "/proc/<pid>/environ (allowlisted)",
            Confidence::High,
        );
    }
    if let Some(de_path) = env.get("GIO_LAUNCHED_DESKTOP_FILE") {
        hints.push::<DesktopEntryPath>(
            PathBuf::from(de_path),
            SourceMethod::ProcEnviron,
            "GIO_LAUNCHED_DESKTOP_FILE env var",
            Confidence::High,
        );
    }
}

/// Surface identity carried straight from the live surface (app_id / title),
/// so a placeholder can capture a new window by matching these exactly.
pub fn push_surface_identity_hints(meta: &Meta, hints: &mut InferredHints) {
    if let Some(app_id) = &meta.app_id {
        hints.push::<AppId>(app_id.clone(), SourceMethod::WaylandSurface, "Wayland app_id / X11 WM_CLASS", Confidence::High);
    }
    if let Some(title) = &meta.title {
        hints.push::<Title>(title.clone(), SourceMethod::WaylandSurface, "Wayland/X11 window title", Confidence::High);
    }
}

/// The `xdg_toplevel_icon_v1` icon the client declared for THIS toplevel.
///
/// `icon` is EXTRA CONTEXT: surface state, readable only from a live window on
/// the compositor thread (`window.icon.toplevel::read`), which is why it arrives
/// as an argument instead of on the [`Meta`] the sampler thread carries. Hints
/// inferred without it simply lack these.
///
/// Only the NAMED half is recorded, as [`XdgIconName`] beside the desktop
/// entry's own [`IconName`]. Attached pixel BUFFERS are deliberately not turned
/// into a hint: hints outlive their window (a placeholder record is kept after
/// the client is gone, and is persisted), and a decoded client buffer is not
/// something to retain there — a consumer that wants those reads them live off
/// the surface with `window.icon.toplevel::read`.
///
/// The name is promoted to [`IconPath`] only as a FALLBACK: if
/// [`push_desktop_hints`] already resolved an icon file, that one stands (the
/// desktop entry is the app's installed identity and matches what a launcher
/// shows). So call this AFTER `push_desktop_hints`.
pub fn push_toplevel_icon_hints(icon: &ToplevelIcon, hints: &mut InferredHints) {
    let Some(name) = &icon.name else { return };
    hints.push::<XdgIconName>(
        name.clone(),
        SourceMethod::WaylandSurface,
        "xdg_toplevel_icon_v1 set_name",
        Confidence::High,
    );
    if hints.has::<IconPath>() {
        return; // desktop entry already resolved one — it is not overridden.
    }
    let Some(resolved) = resolve_icon(name) else { return };
    hints.push::<IconPath>(
        resolved,
        SourceMethod::IconTheme,
        format!("resolved xdg_toplevel_icon name '{name}'"),
        Confidence::High,
    );
}

/// Desktop-entry resolution by app_id: entry path, display name, D-Bus
/// activatability, icon name and resolved icon path.
pub fn push_desktop_hints(meta: &Meta, hints: &mut InferredHints) {
    let Some(app_id) = &meta.app_id else { return };
    let Some(de) = find_by_app_id(app_id) else { return };
    hints.push::<DesktopEntryPath>(
        de.path.clone(),
        SourceMethod::DesktopEntry,
        format!("matched app_id={app_id}"),
        Confidence::High,
    );
    hints.push::<DisplayName>(
        de.name.clone(),
        SourceMethod::DesktopEntry,
        "Name field of desktop entry",
        Confidence::High,
    );
    if de.dbus_activatable {
        hints.push::<DBusActivatable>(
            true,
            SourceMethod::DesktopEntry,
            "DBusActivatable=true",
            Confidence::High,
        );
    }
    if let Some(icon) = &de.icon {
        hints.push::<IconName>(
            icon.clone(),
            SourceMethod::DesktopEntry,
            "Icon field",
            Confidence::High,
        );
        if let Some(resolved) = resolve_icon(icon) {
            hints.push::<IconPath>(
                resolved,
                SourceMethod::IconTheme,
                format!("resolved icon name '{icon}'"),
                Confidence::High,
            );
        }
    }
}

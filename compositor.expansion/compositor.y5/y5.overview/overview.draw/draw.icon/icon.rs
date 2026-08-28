//! Which icon the overview's hover card shows for a window, and where it came
//! from.
//!
//! The `xdg_toplevel_icon_v1` icon wins whenever the client declared one: it is
//! the app's own statement about THIS toplevel, made live, so it is the only
//! source that is right in real time. Everything after it is a fallback, tried
//! in order until one resolves to something that exists.
//!
//! The fallbacks matter most for a window just launched from a placeholder.
//! At first map a client often has not set `app_id` yet, so the introspection
//! captured then resolved no desktop entry and no icon — and the sampler
//! PRESERVES that captured app_id across its refreshes (it cannot read a
//! surface), so the sample can stay iconless long after the app_id has arrived.
//! Reading the surface directly here is what closes that gap.

use smithay::desktop::Window;
use std::path::PathBuf;
use uuid::Uuid;
use compositor_introspection_extraction_window_base::attributes::{IconName, IconPath};
use compositor_introspection_extraction_window_base::desktop::find_by_app_id;
use compositor_introspection_extraction_window_base::icon::resolve as resolve_icon;
use compositor_introspection_extraction_window_base::icon::toplevel;
use compositor_introspection_extraction_window_base::meta::wayland::read_surface_identity;
use compositor_monitor_overview_ui_hover::CardIcon;
use compositor_orchestration_core_state_base::Loop;

/// Which source answered. Reported in the hover card's log line, because "no
/// icon" and "the wrong icon" are different bugs and only this tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSource {
    /// Pixel buffers the client attached over `xdg_toplevel_icon_v1`.
    ToplevelPixels,
    /// An icon name the client declared over the protocol, resolved.
    ToplevelName,
    /// The latest introspection sample's resolved `IconPath`.
    Sample,
    /// The sample's `IconName`, resolved here (the sample recorded a name but
    /// resolved no file — a theme may have arrived since).
    SampleName,
    /// The live `app_id`'s desktop entry, looked up now rather than sampled.
    DesktopEntry,
    /// The live `app_id` used directly as an icon name.
    AppId,
    /// Nothing resolved anywhere.
    None,
    /// Not resolved this pass — the icon already on the card was reused.
    Cached,
}

/// The icon for `window`, and which source produced it.
pub fn resolve(state: &Loop, window: &Window, uuid: Uuid) -> (Option<CardIcon>, IconSource) {
    // 1-2. The protocol, both halves. Preferred: it is live and window-specific.
    if let Some(icon) = toplevel::read(window) {
        if let Some(pixels) = icon.pixels {
            let card = CardIcon::Pixels { width: pixels.width, height: pixels.height, rgba: pixels.rgba };
            return (Some(card), IconSource::ToplevelPixels);
        }
        if let Some(path) = icon.name.as_deref().and_then(resolve_icon) {
            return (Some(path).map(CardIcon::File), IconSource::ToplevelName);
        }
    }
    // 3-4. Whatever introspection has managed to infer so far.
    if let Some(plan) = plan_of(state, uuid) {
        if let Some(path) = plan.0 {
            return (Some(CardIcon::File(path)), IconSource::Sample);
        }
        if let Some(path) = plan.1.as_deref().and_then(resolve_icon) {
            return (Some(CardIcon::File(path)), IconSource::SampleName);
        }
    }
    // 5-7. Straight off the surface, for everything introspection has not caught
    // up with yet. `org.mozilla.firefox` is tried whole and as `firefox`, since
    // themes ship icons under both conventions.
    let (app_id, _, _) = read_surface_identity(window);
    let Some(app_id) = app_id.filter(|a| !a.trim().is_empty()) else {
        return (None, IconSource::None);
    };
    if let Some(path) = find_by_app_id(&app_id).and_then(|de| de.icon).as_deref().and_then(resolve_icon) {
        return (Some(CardIcon::File(path)), IconSource::DesktopEntry);
    }
    for candidate in name_candidates(&app_id) {
        if let Some(path) = resolve_icon(&candidate) {
            return (Some(CardIcon::File(path)), IconSource::AppId);
        }
    }
    (None, IconSource::None)
}

/// `(resolved icon file, icon name)` from the window's latest launch plan —
/// the session plan when it has one, else the persisted one.
fn plan_of(state: &Loop, uuid: Uuid) -> Option<(Option<PathBuf>, Option<String>)> {
    let placeholder = state.inner.placeholder().map.get(&uuid)?.borrow();
    let plan = placeholder.launch_session.as_ref().or(placeholder.launch.as_ref())?;
    Some((plan.current::<IconPath>(), plan.current::<IconName>()))
}

/// Icon-name spellings to try for an app_id: as given, lowercased, and the last
/// dot-segment (`org.mozilla.firefox` → `firefox`). Deduplicated, order kept.
fn name_candidates(app_id: &str) -> Vec<String> {
    let mut out = vec![app_id.to_string()];
    let lower = app_id.to_lowercase();
    if lower != app_id {
        out.push(lower.clone());
    }
    if let Some(tail) = lower.rsplit('.').next() {
        if tail != lower && !tail.is_empty() {
            out.push(tail.to_string());
        }
    }
    out
}

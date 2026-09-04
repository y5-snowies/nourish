//! Foreign-toplevel-management state — BOTH the wlr manager protocol
//! (`zwlr_foreign_toplevel_manager_v1`, hand-rolled) and the modern ext list
//! protocol (`ext_foreign_toplevel_list_v1`, driven through smithay's handler).
//!
//! Docks/taskbars (Waybar `wlr/taskbar`, sfwbar) use the wlr one for list +
//! control; the ext one is list-only (title/app_id/identifier). When the
//! `protocol_foreign` preference is `"enabled"`, both globals are created and
//! advertise the active world's toplevels; when disabled, NEITHER global is
//! offered in the registry — the wlr one is never created and the ext one has its
//! global removed at construction, so clients cannot even bind them. The gate is a
//! startup snapshot read once at boot; there is deliberately NO hot-reload path
//! (reboot to change).
//!
//! This crate holds the world-FREE state; the rim (`wire.base`) reconciles it
//! against the ACTIVE world's window `Space` (so the advertised set is per-world),
//! re-advertising on world switch. The wlr `GlobalDispatch`/`Dispatch` orphan
//! impls + the ext `ForeignToplevelListHandler` impl live in `state.base`.

use std::collections::HashMap;

use smithay::desktop::{Space, Window};
use compositor_support_smithay_state_window_ident::ident;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::{
    ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
};
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::{
    zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
};
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Dispatch, DisplayHandle, GlobalDispatch, Resource};
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListGlobalData, ForeignToplevelListHandler,
    ForeignToplevelListState,
};

/// wlr `state` enum values (fixed by the protocol — see the XML `enum "state"`).
const STATE_MAXIMIZED: u32 = 0;
const STATE_MINIMIZED: u32 = 1;
const STATE_ACTIVATED: u32 = 2;
const STATE_FULLSCREEN: u32 = 3;
/// `fullscreen` state + the (un)set_fullscreen requests appeared in v2.
const FULLSCREEN_SINCE: u32 = 2;

/// Global data for the wlr manager global (no per-global config yet).
#[derive(Debug, Clone, Copy)]
pub struct ForeignManagerGlobalData;

/// User-data on each `zwlr_foreign_toplevel_handle_v1`: the root surface it maps
/// to, so an inbound request (activate/close/…) can be routed back to the window.
#[derive(Debug, Clone)]
pub struct ToplevelHandleData {
    pub surface: WlSurface,
}

/// A control request a dock sent for one toplevel; drained + applied by the rim.
#[derive(Debug, Clone, Copy)]
pub enum ForeignRequest {
    Activate,
    Close,
    Fullscreen(bool),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ToplevelStates {
    maximized: bool,
    minimized: bool,
    activated: bool,
    fullscreen: bool,
}

impl ToplevelStates {
    /// The wl_array payload for the wlr `state` event: native-endian u32 per active
    /// state. Fullscreen is version-gated (v2+).
    fn to_array(self, version: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut push = |v: u32| out.extend_from_slice(&v.to_ne_bytes());
        if self.maximized {
            push(STATE_MAXIMIZED);
        }
        if self.minimized {
            push(STATE_MINIMIZED);
        }
        if self.activated {
            push(STATE_ACTIVATED);
        }
        if self.fullscreen && version >= FULLSCREEN_SINCE {
            push(STATE_FULLSCREEN);
        }
        out
    }
}

#[derive(Debug)]
struct ToplevelEntry {
    title: String,
    app_id: String,
    states: ToplevelStates,
    /// Outputs this toplevel currently overlaps (for wlr output_enter/leave).
    outputs: Vec<Output>,
    /// One wlr handle resource per bound manager instance.
    handles: Vec<ZwlrForeignToplevelHandleV1>,
    /// The ext-protocol handle (smithay manages its per-client instances).
    ext: Option<ForeignToplevelHandle>,
}

/// State of both foreign-toplevel globals.
#[derive(Debug)]
pub struct ForeignToplevel {
    dh: DisplayHandle,
    /// `protocol_foreign == "enabled"` snapshot: gates advertising + callbacks.
    enabled: bool,
    /// `protocol_foreign_all_worlds` snapshot: when set, the rim reconciles against
    /// EVERY world's Space (docks see all windows), not just the active/hosted one.
    all_worlds: bool,
    // wlr: Some only while advertised (enabled at startup); held so the global's
    // lifetime is documented. Never mutated at runtime — this is a boot snapshot.
    #[allow(dead_code)]
    wlr_global: Option<GlobalId>,
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    requests: Vec<(WlSurface, ForeignRequest)>,
    // ext (smithay-driven):
    ext: ForeignToplevelListState,
    // shared mirror keyed by root surface:
    toplevels: HashMap<WlSurface, ToplevelEntry>,
}

impl ForeignToplevel {
    /// Register BOTH globals (always bound). `enabled` gates whether they actually
    /// advertise toplevels + honor requests.
    pub fn new<D>(dh: &DisplayHandle, enabled: bool, all_worlds: bool) -> Self
    where
        D: GlobalDispatch<ZwlrForeignToplevelManagerV1, ForeignManagerGlobalData>
            + GlobalDispatch<ExtForeignToplevelListV1, ForeignToplevelListGlobalData>
            + ForeignToplevelListHandler
            + Dispatch<ExtForeignToplevelHandleV1, ForeignToplevelHandle>
            + 'static,
    {
        // Advertise BOTH globals only when enabled. When disabled we still construct the
        // ext state (the `ForeignToplevelListHandler` accessor must always hand one back)
        // but immediately remove its global, and we never create the wlr global — so a
        // disabled compositor offers NEITHER protocol in the registry. No client is
        // connected at construction, so removing the just-created ext global is invisible.
        let ext = ForeignToplevelListState::new::<D>(dh);
        let wlr_global = if enabled {
            Some(dh.create_global::<D, ZwlrForeignToplevelManagerV1, _>(3, ForeignManagerGlobalData))
        } else {
            dh.remove_global::<D>(ext.global());
            None
        };
        Self {
            dh: dh.clone(),
            enabled,
            all_worlds,
            wlr_global,
            managers: Vec::new(),
            requests: Vec::new(),
            ext,
            toplevels: HashMap::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether to advertise windows from ALL worlds (vs just the hosted one).
    pub fn all_worlds(&self) -> bool {
        self.all_worlds
    }

    /// Live-toggle the all-worlds scope. Unlike `enabled` (which creates/removes the
    /// registry globals and so is boot-only), this only changes which windows the next
    /// reconcile advertises — safe to flip at runtime. The caller re-reconciles after.
    pub fn set_all_worlds(&mut self, all_worlds: bool) {
        self.all_worlds = all_worlds;
    }

    /// The ext list state (for the `ForeignToplevelListHandler` impl in state.base).
    pub fn ext_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.ext
    }

    // ── inbound wlr requests (pushed by the handle Dispatch impl) ──────────────

    pub fn push_request(&mut self, surface: WlSurface, request: ForeignRequest) {
        self.requests.push((surface, request));
    }

    pub fn take_requests(&mut self) -> Vec<(WlSurface, ForeignRequest)> {
        std::mem::take(&mut self.requests)
    }

    // ── wlr resource lifecycle (destructors, from the Dispatch impls) ──────────

    pub fn remove_manager(&mut self, manager: &ZwlrForeignToplevelManagerV1) {
        self.managers.retain(|m| m != manager);
    }

    pub fn remove_handle(&mut self, handle: &ZwlrForeignToplevelHandleV1) {
        for entry in self.toplevels.values_mut() {
            entry.handles.retain(|h| h != handle);
        }
    }

    // ── wlr manager bind: replay every known toplevel onto the new instance ────

    pub fn bind_manager<D>(&mut self, manager: ZwlrForeignToplevelManagerV1)
    where
        D: Dispatch<ZwlrForeignToplevelHandleV1, ToplevelHandleData> + 'static,
    {
        // A manager binding while muted gets an empty list; nothing to replay.
        if self.enabled {
            let dh = self.dh.clone();
            for (surface, entry) in self.toplevels.iter_mut() {
                announce::<D>(&dh, &manager, surface, entry);
            }
        }
        self.managers.push(manager);
    }

    // ── reconcile the mirror against the ACTIVE world's Space (rim, each drain) ─

    /// Diff the tracked toplevels against `spaces` (one window Space per advertised
    /// world — a single active world normally, or every world when `all_worlds` is set),
    /// emitting wlr `toplevel`/`closed` + title/app_id/state/output deltas and the
    /// ext `new_toplevel`/`closed`/title/app_id updates. When muted, tears the
    /// advertisement down (so a live-disable stops showing windows) and returns.
    pub fn reconcile<D>(&mut self, spaces: &[&Space<Window>])
    where
        D: Dispatch<ZwlrForeignToplevelHandleV1, ToplevelHandleData>
            + ForeignToplevelListHandler
            + Dispatch<ExtForeignToplevelHandleV1, ForeignToplevelHandle>
            + 'static,
    {
        if !self.enabled {
            self.teardown();
            return;
        }
        let dh = self.dh.clone();

        // Snapshot the current toplevels across every advertised space (surface +
        // metadata + outputs) up front so the space borrows end before we mutate
        // `self.toplevels`. Outputs are per-space, which is also per-world.
        let current: Vec<(WlSurface, String, String, ToplevelStates, Vec<Output>)> = spaces
            .iter()
            .flat_map(|&space| {
                let outputs = outputs_for(space);
                space.elements().filter_map(move |window| {
                    // The list mirrors what is ON SCREEN, and a Space element is not
                    // automatically that. Both shells can hide without being destroyed —
                    // an X11 unmap clears the `wl_surface` and keeps the `Window`, an xdg
                    // toplevel that commits a null buffer keeps its element with an empty
                    // bbox — and `ident::surface` alone separates only the first of those,
                    // so an X11 window left the dock on hide while a wayland one stayed.
                    if !ident::is_drawn(window) {
                        return None;
                    }
                    let surface = ident::surface(window)?;
                    let names = ident::names(window);
                    let title = names.title.unwrap_or_default();
                    let app_id = names.app_id.unwrap_or_default();
                    Some((surface, title, app_id, read_states(window), outputs.clone()))
                })
            })
            .collect();

        let mut seen: HashMap<WlSurface, ()> = HashMap::with_capacity(current.len());

        for (surface, title, app_id, states, outputs) in current {
            seen.insert(surface.clone(), ());
            match self.toplevels.get_mut(&surface) {
                None => {
                    // New toplevel: announce on every wlr manager + create the ext handle.
                    let mut entry = ToplevelEntry {
                        title,
                        app_id,
                        states,
                        outputs,
                        handles: Vec::new(),
                        ext: None,
                    };
                    for manager in &self.managers {
                        announce::<D>(&dh, manager, &surface, &mut entry);
                    }
                    entry.ext =
                        Some(self.ext.new_toplevel::<D>(entry.title.clone(), entry.app_id.clone()));
                    self.toplevels.insert(surface, entry);
                }
                Some(entry) => {
                    // Existing: emit only what changed, then one `done` per protocol.
                    let mut wlr_changed = false;
                    let mut meta_changed = false; // title/app_id — drives ext too
                    if entry.title != title {
                        entry.title = title.clone();
                        for h in &entry.handles {
                            h.title(title.clone());
                        }
                        if let Some(e) = &entry.ext {
                            e.send_title(&title);
                        }
                        wlr_changed = true;
                        meta_changed = true;
                    }
                    if entry.app_id != app_id {
                        entry.app_id = app_id.clone();
                        for h in &entry.handles {
                            h.app_id(app_id.clone());
                        }
                        if let Some(e) = &entry.ext {
                            e.send_app_id(&app_id);
                        }
                        wlr_changed = true;
                        meta_changed = true;
                    }
                    if entry.outputs != outputs {
                        let added: Vec<Output> =
                            outputs.iter().filter(|o| !entry.outputs.contains(o)).cloned().collect();
                        let removed: Vec<Output> =
                            entry.outputs.iter().filter(|o| !outputs.contains(o)).cloned().collect();
                        for h in &entry.handles {
                            send_outputs(&dh, h, &added, true);
                            send_outputs(&dh, h, &removed, false);
                        }
                        entry.outputs = outputs;
                        wlr_changed = true;
                    }
                    if entry.states != states {
                        entry.states = states;
                        for h in &entry.handles {
                            h.state(states.to_array(h.version()));
                        }
                        wlr_changed = true;
                    }
                    if wlr_changed {
                        for h in &entry.handles {
                            h.done();
                        }
                    }
                    if meta_changed {
                        if let Some(e) = &entry.ext {
                            e.send_done();
                        }
                    }
                }
            }
        }

        // Anything no longer in the active world's Space closed (or moved worlds):
        // notify + drop. Collected first to avoid a nested borrow of `self`.
        let gone: Vec<WlSurface> =
            self.toplevels.keys().filter(|s| !seen.contains_key(*s)).cloned().collect();
        for surface in gone {
            if let Some(entry) = self.toplevels.remove(&surface) {
                close_entry(&mut self.ext, &entry);
            }
        }
    }

    /// Close + drop every advertised toplevel (used when the protocol is muted).
    fn teardown(&mut self) {
        for (_surface, entry) in self.toplevels.drain() {
            close_entry(&mut self.ext, &entry);
        }
    }
}

/// Close one entry on both protocols.
fn close_entry(ext: &mut ForeignToplevelListState, entry: &ToplevelEntry) {
    for h in &entry.handles {
        h.closed();
    }
    if let Some(ext_handle) = &entry.ext {
        ext.remove_toplevel(ext_handle);
    }
}

/// Create a wlr handle for `entry` on `manager`'s client and send its initial state.
fn announce<D>(
    dh: &DisplayHandle,
    manager: &ZwlrForeignToplevelManagerV1,
    surface: &WlSurface,
    entry: &mut ToplevelEntry,
) where
    D: Dispatch<ZwlrForeignToplevelHandleV1, ToplevelHandleData> + 'static,
{
    let Ok(client) = dh.get_client(manager.id()) else {
        return;
    };
    let data = ToplevelHandleData { surface: surface.clone() };
    let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, D>(dh, manager.version(), data)
    else {
        warn!("foreign-toplevel: failed to create handle resource");
        return;
    };
    manager.toplevel(&handle);
    handle.title(entry.title.clone());
    handle.app_id(entry.app_id.clone());
    send_outputs(dh, &handle, &entry.outputs, true);
    handle.state(entry.states.to_array(handle.version()));
    handle.done();
    entry.handles.push(handle);
}

/// Send `output_enter`/`output_leave` for `outputs` to one handle, resolving the
/// handle client's own `wl_output` resources for each (a client sees a toplevel's
/// output only through the `wl_output` it bound).
fn send_outputs(dh: &DisplayHandle, handle: &ZwlrForeignToplevelHandleV1, outputs: &[Output], enter: bool) {
    let Ok(client) = dh.get_client(handle.id()) else {
        return;
    };
    for output in outputs {
        for wl_output in output.client_outputs(&client) {
            if enter {
                handle.output_enter(&wl_output);
            } else {
                handle.output_leave(&wl_output);
            }
        }
    }
}

/// The outputs a window is on: every output of its world's space.
///
/// Not a geometric test, and not a visibility one. In y5 no window belongs to an
/// output — every monitor renders the same world through its own camera, so any
/// monitor can bring any window into view on the next frame, and a window currently
/// outside a camera is still on that monitor's output. This deliberately mirrors the
/// `wl_output` membership `Orchestrator::refresh_space` publishes, so a dock and the
/// client itself are never told two different things.
///
/// The former version intersected the window's space geometry with each output's,
/// which reads as obvious and is wrong twice over: Space positions are y5-WORLD
/// coordinates, so a window at negative world x/y matched no output and docks were
/// told it was on none, and with several monitors the outputs tile side by side so a
/// window's world x picked which monitor it was reported on.
fn outputs_for(space: &Space<Window>) -> Vec<Output> {
    space.outputs().cloned().collect()
}

/// The wlr states, in this module's own shape. `window.ident` answers the question
/// for either shell (xdg pending state, or the X11 `_NET_WM_STATE` predicates); this
/// only re-labels the answer, because `ToplevelStates` carries the wl_array encoding
/// the protocol wants and is private to this crate.
fn read_states(window: &Window) -> ToplevelStates {
    let s = ident::states(window);
    ToplevelStates {
        maximized: s.maximized,
        minimized: s.minimized,
        activated: s.activated,
        fullscreen: s.fullscreen,
    }
}

//! `xdg_toplevel_drag_v1` — drag a toplevel along with a drag-and-drop.
//!
//! This is what browsers bind to detach a tab into its own window: the client
//! starts a normal `wl_data_device.start_drag`, then hands us a toplevel via
//! [`attach`](xdg_toplevel_drag_v1::Request::Attach) and we move that toplevel
//! with the cursor for the rest of the drag, "as if the client called
//! `xdg_toplevel.move`".
//!
//! Unlike `wire.session`, no XML is vendored here: `wayland-protocols` ships
//! this staging protocol AND generates bindings for it (behind its `staging`
//! feature, which smithay already enables), so the types come straight off
//! `smithay::reexports`.
//!
//! The `Dispatch`/`GlobalDispatch` impls live in `dispatch.state/state.base`
//! alongside the other hand-rolled protocols, since that is where
//! `delegate_dispatch2!(Dispatch)` makes the bounds provable.
//!
//! ## Why the move is deferred rather than applied here
//!
//! `Dispatch` owns no `Space` — window placement lives in the orchestration
//! layer. So the grab (`state.grab/grab.drag.state`) does not move anything
//! itself; it records the wanted world position and the orchestration drain
//! applies it, exactly like `pending_restoration`.
//!
//! ## Coordinates
//!
//! `attach`'s offset is surface-local **logical** units. y5-world is also
//! logical (the camera is applied at render time, not at storage time), so the
//! offset subtracts directly off the world-space cursor position with no
//! projection — see `transform.translate`'s note that storage positions must
//! not be routed back through `Transform`.

use std::collections::HashMap;
use std::sync::Mutex;

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
pub use smithay::reexports::wayland_protocols::xdg::toplevel_drag::v1::server::{
    xdg_toplevel_drag_manager_v1::{self, Error as ManagerError, XdgToplevelDragManagerV1},
    xdg_toplevel_drag_v1::{self, Error as DragError, XdgToplevelDragV1},
};
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    DataInit, Dispatch, DisplayHandle, GlobalDispatch, Resource,
};
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::get_parent;
use smithay::wayland::shell::xdg::XdgShellSurfaceUserData;

/// Only version 1 exists.
pub const VERSION: u32 = 1;

/// What `attach` recorded: the toplevel that follows the cursor, and where the
/// cursor sits inside it.
#[derive(Debug, Clone)]
pub struct Attached {
    pub surface: WlSurface,
    /// Surface-local cursor offset, logical units.
    pub offset: Point<i32, Logical>,
}

/// User data hung off every `xdg_toplevel_drag_v1`.
#[derive(Debug)]
pub struct ToplevelDragData {
    /// The source this drag was created for — the key the DnD start looks up.
    pub source: WlDataSource,
    pub attached: Mutex<Option<Attached>>,
    /// True while the underlying DnD grab is running. `destroy` in that window
    /// is an `ongoing_drag` protocol error.
    pub active: Mutex<bool>,
    /// The toplevel the drag STARTED on — the subsurface root of the pointer's
    /// focus when the grab was installed. Read by [`live`](Self::live).
    origin: Mutex<Option<WlSurface>>,
}

impl ToplevelDragData {
    fn new(source: WlDataSource) -> Self {
        Self {
            source,
            attached: Mutex::new(None),
            active: Mutex::new(false),
            origin: Mutex::new(None),
        }
    }

    /// The toplevel this drag is actually carrying.
    ///
    /// Drops the attachment if the toplevel has since gone away — the client is
    /// allowed to destroy the dragged window mid-drag to signal "snap this back
    /// into the parent".
    ///
    /// Also answers `None` when the attached toplevel IS the one the drag
    /// started on, which is not a tear-off at all: a browser whose window holds
    /// a single tab has nothing to detach that tab INTO, so it attaches the
    /// window itself. Carrying it would drag the whole window around by its tab
    /// strip — a window move nobody asked for, off a gesture that produced no
    /// new window. Answering `None` makes this an ordinary drag-and-drop, which
    /// is what it is; the tab can still be dropped into another window.
    ///
    /// Everything that asks "is a window following the cursor" goes through
    /// here — the carry itself, the self-focus suppression, the pointer hit-test
    /// exclusion, the initial-map placement. `ToplevelDragState::is_carrying`
    /// deliberately does NOT: dropping that single tab into another window
    /// destroys this toplevel, and that destroy must still leave no placeholder.
    pub fn live(&self) -> Option<Attached> {
        // Scoped so the `attached` guard is released before `origin` is taken. No
        // two of these locks are ever held at once anywhere in this file, which is
        // what makes a re-entrant deadlock structurally impossible rather than
        // merely absent — the compositor is single-threaded, so a lock that ever
        // did block would block forever.
        let attached = {
            let mut slot = self.attached.lock().ok()?;
            if slot.as_ref().is_some_and(|a| !a.surface.is_alive()) {
                *slot = None;
            }
            slot.clone()?
        };
        if self.origin.lock().ok()?.as_ref() == Some(&attached.surface) {
            return None;
        }
        Some(attached)
    }

    pub fn set_active(&self, on: bool) {
        if let Ok(mut a) = self.active.lock() {
            *a = on;
        }
    }

    /// Record where the drag started, once, as the grab is installed. Stored as
    /// the subsurface root, since the pointer usually sits on a child surface
    /// and `attach` always names a toplevel.
    pub fn set_origin(&self, surface: Option<&WlSurface>) {
        // `root_of` walks the subsurface tree through smithay's `with_states`,
        // which is itself non-reentrant — so it is resolved BEFORE the lock is
        // taken. Nothing foreign is ever called while one of these guards is held.
        let root = surface.map(root_of);
        if let Ok(mut o) = self.origin.lock() {
            *o = root;
        }
    }
}

/// The root of `surface`'s subsurface tree.
fn root_of(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

/// Registry of the drags created so far, keyed by their `wl_data_source`.
///
/// The protocol allows at most one drag per source (`invalid_source`), and the
/// DnD start path needs to answer "does this source have a toplevel attached?",
/// which is what this map is for.
#[derive(Debug, Default)]
pub struct ToplevelDragState {
    drags: HashMap<ObjectId, XdgToplevelDragV1>,
    /// Positions the grab queued for the window it is carrying, in y5-world
    /// logical coordinates. The grab cannot place the window itself (`Dispatch`
    /// owns no `Space`), so the frame hook applies these.
    pub moves: Vec<(WlSurface, Point<f64, Logical>)>,
    /// Toplevels a carry has just finished with. Drained into the window
    /// lifecycle queue, which re-syncs their placeholder to where the carry
    /// left them.
    pub settled: Vec<WlSurface>,
    /// Toplevels abandoned by a drag the COMPOSITOR cancelled (a world switch
    /// under a live carry). The destroy that follows must leave no placeholder,
    /// and by then the carry is over — so the verdict is latched here.
    pub abandoned: Vec<WlSurface>,
}

impl ToplevelDragState {
    /// The drag object registered for `source`, if any and still alive.
    pub fn for_source(&self, source: &WlDataSource) -> Option<&XdgToplevelDragV1> {
        self.drags.get(&source.id()).filter(|d| d.is_alive())
    }

    /// The surface currently being carried by a live drag, if any.
    ///
    /// The pointer hit test must EXCLUDE this surface. The carried window sits
    /// under the cursor for the whole drag, so leaving it in the hit test makes
    /// it the drop target on every motion; merely discarding that focus is not
    /// enough either, because then the drag has no target at all and the window
    /// underneath — the one the user is actually aiming at — never sees an
    /// enter or a drop. Excluding it here lets the hit fall through to it.
    pub fn carried_surface(&self) -> Option<WlSurface> {
        self.drags
            .values()
            .filter(|d| d.is_alive())
            .filter_map(|d| d.data::<ToplevelDragData>())
            .filter(|d| d.active.lock().is_ok_and(|a| *a))
            .find_map(|d| d.live().map(|a| a.surface))
    }

    /// The carried surface together with its attach offset.
    ///
    /// Needed wherever the carried window has to be re-anchored without a
    /// pointer motion to hang it off — a camera pan or zoom moves the world
    /// under a stationary cursor, and emits no motion event at all.
    pub fn carried_with_offset(&self) -> Option<(WlSurface, Point<i32, Logical>)> {
        self.drags
            .values()
            .filter(|d| d.is_alive())
            .filter_map(|d| d.data::<ToplevelDragData>())
            .filter(|d| d.active.lock().is_ok_and(|a| *a))
            .find_map(|d| d.live().map(|a| (a.surface, a.offset)))
    }

    /// Mark every registered drag as finished.
    ///
    /// Called the moment the DnD ends (drop or cancel), not just when the
    /// pointer grab is torn down. A client is told the drag ended via
    /// `dnd_drop_performed`/`cancelled` and is then entitled to `destroy` the
    /// drag object immediately; if `active` were still set at that point we
    /// would answer a legal request with `ongoing_drag`, which kills the
    /// client. Clearing here makes that unreachable regardless of how the
    /// grab teardown and the client's next request interleave.
    pub fn deactivate_all(&mut self) {
        for drag in self.drags.values() {
            if let Some(d) = drag.data::<ToplevelDragData>() {
                d.set_active(false);
            }
        }
    }

    /// Is `surface` being carried by a drag that is running RIGHT NOW?
    ///
    /// Asked when a toplevel is destroyed, to decide whether it leaves a
    /// placeholder tile behind. Nothing is remembered and nothing is marked —
    /// the answer is only ever about the current instant, because that is
    /// exactly where the distinction lives:
    ///
    /// - Dropped into another toplevel: the receiving client adopts the tab and
    ///   gets rid of the carrier **during** the drag — the spec's dock flow is
    ///   "delete or unmap the dragged top-level" mid-drag, which is also why a
    ///   re-`attach` afterwards is legal. Destroyed while carried → no
    ///   placeholder; nothing was closed from the user's point of view, and a
    ///   tile here would mark a window that never really existed.
    /// - Dropped anywhere else: the window outlives the drag and is a real
    ///   window. Whenever it is closed later the drag is long over, so this
    ///   answers false and it leaves a placeholder like anything else.
    /// - Hovered over a drop target and then pulled off again on the same drag:
    ///   nothing was destroyed, so this is never consulted — and if that window
    ///   is closed later it is the previous case.
    ///
    /// Deliberately reads the raw attachment rather than [`ToplevelDragData::live`]:
    /// the caller is asking about a surface that is in the middle of dying, and
    /// the liveness filter would answer `None` for precisely the case this
    /// exists to catch.
    pub fn is_carrying(&self, surface: &WlSurface) -> bool {
        self.drags
            .values()
            .filter(|d| d.is_alive())
            .filter_map(|d| d.data::<ToplevelDragData>())
            .filter(|d| d.active.lock().is_ok_and(|a| *a))
            .any(|d| {
                d.attached.lock().is_ok_and(|s| s.as_ref().is_some_and(|a| &a.surface == surface))
            })
    }

    /// Drop entries whose drag object or source died.
    ///
    /// The source is checked too, not just the drag: the key is a
    /// `wl_data_source` object id, and wayland reuses ids once an object is
    /// gone. A dead source whose drag outlived it would leave the id pointing
    /// at a stale drag, and the next client to be handed that id would get a
    /// spurious `invalid_source` — i.e. be killed for someone else's object.
    fn sweep(&mut self) {
        self.drags.retain(|_, d| {
            d.is_alive() && d.data::<ToplevelDragData>().is_some_and(|s| s.source.is_alive())
        });
    }
}

/// The compositor state's hook for a toplevel-drag move.
///
/// The grab lives upstream of `dispatch.state/state.base` (state.base has to
/// construct it, so the dependency cannot run the other way), and it has no
/// `Space` to write to regardless. So it calls this instead, and the concrete
/// state queues the move for the orchestration drain to apply.
pub trait ToplevelDragHost {
    /// Queue a carried window's new position. See [`ToplevelDragState::moves`].
    ///
    /// Move `surface`'s window to `location`, given in **y5-world** logical
    /// coordinates — already the space `Space::map_element` stores, so no
    /// camera projection may be applied on the way.
    fn queue_toplevel_drag_move(&mut self, surface: WlSurface, location: Point<f64, Logical>);
}

pub fn create_global<W>(dh: &DisplayHandle)
where
    W: GlobalDispatch<XdgToplevelDragManagerV1, ()> + 'static,
{
    dh.create_global::<W, XdgToplevelDragManagerV1, ()>(VERSION, ());
    info!("drag: xdg_toplevel_drag_manager_v1 global advertised");
}

/// `get_xdg_toplevel_drag` — one drag per `wl_data_source`, else `invalid_source`.
pub fn dispatch_manager<D>(
    state: &mut ToplevelDragState,
    manager: &XdgToplevelDragManagerV1,
    request: xdg_toplevel_drag_manager_v1::Request,
    di: &mut DataInit<'_, D>,
) where
    D: Dispatch<XdgToplevelDragV1, ToplevelDragData> + 'static,
{
    let xdg_toplevel_drag_manager_v1::Request::GetXdgToplevelDrag { id, data_source } = request
    else {
        return;
    };

    state.sweep();
    // `invalid_source` is likewise not raised: the newest drag for a source
    // wins. Same trade as the other two — killing a client over bookkeeping is
    // never worth it, and we can always tell what the client wants.
    if state.for_source(&data_source).is_some() {
        info!("drag: second drag for one source — replacing the previous one");
    }

    let key = data_source.id();
    let drag = di.init(id, ToplevelDragData::new(data_source));
    state.drags.insert(key, drag);
    info!("drag: client took an xdg_toplevel_drag_v1 for a data source");
}

/// `attach` / `destroy` on a live drag object.
pub fn dispatch_drag(
    state: &mut ToplevelDragState,
    drag: &XdgToplevelDragV1,
    request: xdg_toplevel_drag_v1::Request,
    data: &ToplevelDragData,
) {
    match request {
        xdg_toplevel_drag_v1::Request::Attach { toplevel, x_offset, y_offset } => {
            // NOT raising `toplevel_attached`, deliberately — same leniency as
            // `wire.session`: the error kills the client, and taking the newer
            // attachment is harmless to us.
            //
            // It is also the wrong answer here. The spec's dock flow is "delete
            // OR UNMAP the dragged top-level", then attach a new one to tear
            // again. An unmapped surface is still `is_alive()`, so testing
            // liveness treats a legal re-attach as a protocol violation — which
            // is what killed Chrome mid tab-drag. Detecting "unmapped" from a
            // bare `wl_surface` is not something this crate can do reliably, so
            // the newest attachment simply wins.
            if data.live().is_some() {
                info!("drag: re-attach over a live toplevel — taking the new one");
            }
            let Some(surface) = surface_of(&toplevel) else {
                warn!("drag: attach with an xdg_toplevel carrying no surface — ignored");
                return;
            };
            if let Ok(mut slot) = data.attached.lock() {
                *slot = Some(Attached { surface, offset: Point::from((x_offset, y_offset)) });
            }
            info!("drag: toplevel attached at offset ({x_offset}, {y_offset})");
        }
        xdg_toplevel_drag_v1::Request::Destroy => {
            // `ongoing_drag` is specified for a destroy before the drag ends, but
            // it is not raised for the same reason as above: it kills the client,
            // and dropping the drag early costs us nothing — we simply stop
            // carrying. Racing our own `dnd_finished` against the client's
            // `destroy` must never be fatal to a browser.
            if data.active.lock().is_ok_and(|a| *a) {
                info!("drag: destroy while still running — dropping the drag instead of erroring");
            }
            // Removed by object identity rather than by `data.source.id()`:
            // if the source died first its id may already have been reused, and
            // removing by that key would evict somebody else's entry.
            state.drags.retain(|_, d| d != drag);
            state.sweep();
        }
        _ => {}
    }
}

/// The `wl_surface` behind a raw `xdg_toplevel` resource. Smithay hangs
/// `XdgShellSurfaceUserData` off every toplevel it creates, so this is present
/// for any toplevel that came through xdg-shell (i.e. all of them).
fn surface_of(toplevel: &XdgToplevel) -> Option<WlSurface> {
    toplevel.data::<XdgShellSurfaceUserData>().map(|d| d.wl_surface().clone())
}

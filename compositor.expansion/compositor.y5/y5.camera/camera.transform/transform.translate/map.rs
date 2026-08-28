//! World ↔ surface-local for ANY surface of a window — the root or one of its
//! subsurfaces — through the SAME fit the renderer and the hit test apply.
//!
//! The pointer-constraint paths (confine bounds, the unlock-restoration warp)
//! used to build their own coordinates from raw `element_location` / `geometry`:
//! the geometry origin taken for the surface origin (off by the CSD inset), no
//! fit scale or letterbox offset (wrong for every window whose content does
//! not match its slot — viewport, fractional scale, an ignored fullscreen
//! configure), and a window lookup that only matched the ROOT surface, so a
//! game constraining the subsurface it presents on resolved to a 0x0 window at
//! the world origin. One mapping, derived exactly as `hit.rs` derives its
//! inversion, replaces all of that.

use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::desktop::{Space, Window};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Size};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::compositor::{
    get_parent, with_states, with_surface_tree_downward, SubsurfaceCachedState, TraversalAction,
};

use crate::fit::window_fit;
use crate::slot;

/// A surface's placement in the world: `world = fit_surf + (local + offset) · fit_s`.
#[derive(Debug, Clone)]
pub struct SurfaceMap {
    pub window: Window,
    fit_surf: (f64, f64),
    fit_sx: f64,
    fit_sy: f64,
    /// The surface's origin within its root's surface-local space (0 for the root).
    offset: Point<i32, Logical>,
    /// The surface's own logical extent (`SurfaceView.dst`: viewport / scale applied).
    pub extent: Size<i32, Logical>,
}

impl SurfaceMap {
    /// A world point in this surface's own local coordinates.
    pub fn to_local(&self, world: Point<f64, Logical>) -> Point<f64, Logical> {
        Point::from((
            (world.x - self.fit_surf.0) / self.fit_sx - self.offset.x as f64,
            (world.y - self.fit_surf.1) / self.fit_sy - self.offset.y as f64,
        ))
    }

    /// Where a surface-local point of this surface is displayed in the world.
    pub fn to_world(&self, local: Point<f64, Logical>) -> Point<f64, Logical> {
        Point::from((
            self.fit_surf.0 + (local.x + self.offset.x as f64) * self.fit_sx,
            self.fit_surf.1 + (local.y + self.offset.y as f64) * self.fit_sy,
        ))
    }
}

/// The surface's logical extent as the renderer sees it.
fn view_dst(surface: &WlSurface) -> Option<Size<i32, Logical>> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<RendererSurfaceStateUserData>()
            .and_then(|m| m.lock().ok().and_then(|s| s.view()))
            .map(|v| v.dst)
    })
}

/// Resolve `surface` (root or subsurface) to its window and fit. `None` when no
/// mapped window owns it.
pub fn surface_map(space: &Space<Window>, surface: &WlSurface) -> Option<SurfaceMap> {
    let mut root = surface.clone();
    while let Some(p) = get_parent(&root) {
        root = p;
    }
    let window = space
        .elements()
        .find(|w| w.wl_surface().as_deref() == Some(&root))?
        .clone();
    let elem_loc = space.element_location(&window)?;
    let geom = window.geometry();

    // The slot, resolved exactly as `hit.rs` / `window.draw.frame::scene` resolve it.
    let cfg = compositor_model_environment_config_base::base::get();
    let slot_size = slot::size_of(&window);

    let (fit_surf, fit_sx, fit_sy) = match slot_size {
        // No slot decided: native placement, the surface origin at `elem_loc - geom.loc`.
        None => (((elem_loc.x - geom.loc.x) as f64, (elem_loc.y - geom.loc.y) as f64), 1.0, 1.0),
        Some(slot_size) => {
            let dst = view_dst(&root).unwrap_or(geom.size);
            let stretch = slot::resize_stretching(&window, geom.size);
            let f = window_fit(elem_loc, geom, dst, window.bbox(), slot_size, cfg.window_subsurface_shrinks, stretch);
            (f.fit_surf, f.fit_sx, f.fit_sy)
        }
    };

    // The subsurface's origin within the root's local space: the sum of the
    // `wl_subsurface` positions down the tree.
    let mut offset = Point::from((0, 0));
    if root != *surface {
        with_surface_tree_downward(
            &root,
            Point::<i32, Logical>::from((0, 0)),
            |s, states, parent_loc| {
                let loc = if *s == root {
                    *parent_loc
                } else {
                    *parent_loc + states.cached_state.get::<SubsurfaceCachedState>().current().location
                };
                if s == surface {
                    offset = loc;
                }
                TraversalAction::DoChildren(loc)
            },
            |_, _, _| {},
            |_, _, _| true,
        );
    }

    let extent = view_dst(surface).unwrap_or(geom.size);
    Some(SurfaceMap { window, fit_surf, fit_sx, fit_sy, offset, extent })
}

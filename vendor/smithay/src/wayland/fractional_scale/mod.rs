//! Utilities for handling the `wp_fractional_scale` protocol
//!
//! ## How to use it
//!
//! ### Initialization
//!
//! To initialize this implementation create the [`FractionalScaleManagerState`], store it inside your `State` struct
//! and implement the [`FractionalScaleHandler`], as shown in this example:
//!
//! ```
//! use smithay::wayland::compositor;
//! use smithay::reexports::wayland_server::protocol::wl_surface;
//! use smithay::wayland::fractional_scale::{
//!     self,
//!     FractionalScaleManagerState,
//!     FractionalScaleHandler,
//! };
//!
//! # struct State { fractional_scale_manager_state: FractionalScaleManagerState }
//! # let mut display = wayland_server::Display::<State>::new().unwrap();
//! // Create the compositor state
//! let fractional_scale_manager_state = FractionalScaleManagerState::new::<State>(
//!     &display.handle(),
//! );
//!
//! // insert the FractionalScaleManagerState into your state
//! // ..
//!
//! // implement the necessary traits
//! impl FractionalScaleHandler for State {
//!    fn new_fractional_scale(&mut self, surface: wl_surface::WlSurface) {
//!        // If you set preferred fractional scale for this surface before,
//!        // then you don't need to do anything here.
//!    }
//! }
//!
//! smithay::delegate_dispatch2!(State);
//!
//! // You're now ready to go!
//! ```
//!
//! ### Use the fractional scale state
//!
//! Whenever the fractional scale for a surface changes set the preferred
//! fractional scale like shown in the example:
//!
//! ```no_run
//! # use wayland_server::{backend::ObjectId, protocol::wl_surface, Resource};
//! use smithay::wayland::compositor;
//! use smithay::wayland::fractional_scale;
//! # struct State;
//! # let mut display = wayland_server::Display::<State>::new().unwrap();
//! # let dh = display.handle();
//! # let surface = wl_surface::WlSurface::from_id(&dh, ObjectId::null()).unwrap();
//! compositor::with_states(&surface, |states| {
//!     fractional_scale::with_fractional_scale(states, |fractional_scale| {
//!         // set the preferred scale for the surface
//!         fractional_scale.set_preferred_scale(1.25);
//!     });
//! })
//! ```

use std::cell::RefCell;

use wayland_protocols::wp::fractional_scale::v1::server::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1,
};
use wayland_server::{
    Dispatch, DisplayHandle, GlobalDispatch, Resource, Weak, backend::GlobalId, protocol::wl_surface,
};

use super::compositor::{SurfaceData, with_states};

use crate::wayland::{Dispatch2, GlobalData, GlobalDispatch2};
use std::sync::Mutex;

/// State of the wp_fractional_scale_manager_v1 Global
#[derive(Debug)]
pub struct FractionalScaleManagerState {
    global: GlobalId,
}

impl FractionalScaleManagerState {
    /// Create new [`wp_fraction_scale_manager`](wayland_protocols::wp::fractional_scale::v1::server::wp_fractional_scale_manager_v1) global.
    pub fn new<D>(display: &DisplayHandle) -> FractionalScaleManagerState
    where
        D: GlobalDispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, GlobalData>
            + Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, GlobalData>
            + Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, FractionalScaleData>
            + 'static,
        D: FractionalScaleHandler,
    {
        FractionalScaleManagerState {
            global: display
                .create_global::<D, wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _>(
                    1, GlobalData,
                ),
        }
    }

    /// Returns the fractional scale manager global.
    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

/// Clients this global is hidden from.
///
/// `wp_fractional_scale_v1` asks the compositor what scale a surface should render at,
/// and whoever bound it acts on the answer. That is right for an application drawing its
/// own content and wrong for a client that merely FORWARDS other applications' windows —
/// an Xwayland server applies the scale to X clients that never asked and cannot be
/// consulted.
///
/// A filter rather than a refused bind: a global a client cannot see is one it never
/// binds, so no traffic is generated to be discarded.
static VISIBILITY_FILTER: Mutex<Option<Box<dyn Fn(&wayland_server::Client) -> bool + Send + Sync>>> =
    Mutex::new(None);

/// Restrict which clients are shown `wp_fractional_scale_manager_v1`.
///
/// The filter returns `true` for a client that may see the global. Replaces any
/// previously installed filter; with none installed every client sees it.
pub fn set_visibility_filter(
    filter: impl Fn(&wayland_server::Client) -> bool + Send + Sync + 'static,
) {
    *VISIBILITY_FILTER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(filter));
}

impl<D> GlobalDispatch2<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, D> for GlobalData
where
    D: Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, GlobalData>
        + Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, FractionalScaleData>,
    D: FractionalScaleHandler,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &wayland_server::Client,
        resource: wayland_server::New<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
        data_init: &mut wayland_server::DataInit<'_, D>,
    ) {
        data_init.init(resource, GlobalData);
    }

    fn can_view(&self, client: &wayland_server::Client) -> bool {
        VISIBILITY_FILTER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_none_or(|filter| filter(client))
    }
}

impl<D> Dispatch2<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, D> for GlobalData
where
    D: Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, FractionalScaleData>,
    D: FractionalScaleHandler,
{
    fn request(
        &self,
        state: &mut D,
        _client: &wayland_server::Client,
        _resource: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        request: <wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut wayland_server::DataInit<'_, D>,
    ) {
        match request {
            wp_fractional_scale_manager_v1::Request::Destroy => {
                // All is already handled by our destructor
            }
            wp_fractional_scale_manager_v1::Request::GetFractionalScale { id, surface } => {
                let already_has_fractional_scale = with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<FractionalScaleStateUserData>()
                        .map(|v| v.borrow().fractional_scale.is_some())
                        .unwrap_or(false)
                });

                if already_has_fractional_scale {
                    surface.post_error(
                        wp_fractional_scale_manager_v1::Error::FractionalScaleExists,
                        "the surface already has a fractional_scale object associated".to_string(),
                    );
                    return;
                }

                let fractional_scale: wp_fractional_scale_v1::WpFractionalScaleV1 =
                    data_init.init(id, FractionalScaleData(surface.downgrade()));

                with_states(&surface, move |states| {
                    with_fractional_scale(states, move |data| {
                        // Send the scale that the user may have pre-filled for this surface.
                        if let Some(scale) = data.preferred_scale {
                            fractional_scale.preferred_scale(f64::round(scale * 120.0) as u32);
                        }
                        data.fractional_scale = Some(fractional_scale);
                    });
                });
                state.new_fractional_scale(surface);
            }
            _ => unreachable!(),
        }
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct FractionalScaleData(Weak<wl_surface::WlSurface>);

impl<D> Dispatch2<wp_fractional_scale_v1::WpFractionalScaleV1, D> for FractionalScaleData
where
    D: FractionalScaleHandler,
{
    fn request(
        &self,
        _state: &mut D,
        _client: &wayland_server::Client,
        _resource: &wp_fractional_scale_v1::WpFractionalScaleV1,
        request: <wp_fractional_scale_v1::WpFractionalScaleV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut wayland_server::DataInit<'_, D>,
    ) {
        match request {
            wp_fractional_scale_v1::Request::Destroy => {
                if let Ok(surface) = self.0.upgrade() {
                    with_states(&surface, |states| {
                        states
                            .data_map
                            .get::<FractionalScaleStateUserData>()
                            .and_then(|v| v.borrow_mut().fractional_scale.take());
                    })
                }
            }
            _ => unreachable!(),
        }
    }
}

/// Fractional scale handler type
pub trait FractionalScaleHandler {
    /// A new fractional scale was instantiated
    fn new_fractional_scale(&mut self, _surface: wl_surface::WlSurface) {}
}

/// Type stored in WlSurface states data_map
///
/// ```rs
/// compositor::with_states(surface, |states| {
///     let data = states.data_map.get::<FractionalScaleStateUserData>();
/// });
/// ```
pub type FractionalScaleStateUserData = RefCell<FractionalScaleState>;

/// State for the fractional scale of a surface
#[derive(Debug, Default)]
pub struct FractionalScaleState {
    /// The fractional scale object, if one exists for this surface.
    fractional_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    /// Preferred fractional scale for this surface.
    preferred_scale: Option<f64>,
}

impl FractionalScaleState {
    /// Set the preferred scale
    pub fn set_preferred_scale(&mut self, scale: f64) {
        if self.preferred_scale != Some(scale) {
            if let Some(obj) = &self.fractional_scale {
                obj.preferred_scale(f64::round(scale * 120.0) as u32);
            }
            self.preferred_scale = Some(scale);
        }
    }

    /// Returns the current preferred scale
    pub fn preferred_scale(&self) -> Option<f64> {
        self.preferred_scale
    }
}

/// Run a closure on the [`FractionalScaleState`] of a [`WlSurface`](wl_surface::WlSurface)
pub fn with_fractional_scale<F, T>(states: &SurfaceData, f: F) -> T
where
    F: FnOnce(&mut FractionalScaleState) -> T,
{
    let mut fractional_scale = states
        .data_map
        .get_or_insert(FractionalScaleStateUserData::default)
        .borrow_mut();

    f(&mut fractional_scale)
}

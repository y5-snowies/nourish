use std::{
    any::Any,
    cell::RefCell,
    os::fd::{AsFd, OwnedFd},
    sync::Mutex,
};
use tracing::{debug, error};

use wayland_server::{
    DisplayHandle, Resource,
    backend::ClientId,
    protocol::{
        wl_data_source::{self, WlDataSource},
        wl_surface::WlSurface,
    },
};

use crate::input::{
    Seat,
    dnd::{DndAction, Source, SourceMetadata},
};
use crate::utils::{IsAlive, alive_tracker::AliveTracker};
use crate::wayland::Dispatch2;
use crate::wayland::selection::SelectionTarget;
use crate::wayland::selection::offer::OfferReplySource;
use crate::wayland::selection::seat_data::SeatData;
use crate::wayland::selection::source::{CompositorSelectionProvider, SelectionSourceProvider};

use super::DataDeviceHandler;

#[doc(hidden)]
#[derive(Debug)]
pub struct DataSourceUserData {
    pub(crate) inner: Mutex<SourceMetadata>,
    alive_tracker: AliveTracker,
    display_handle: DisplayHandle,
}

impl DataSourceUserData {
    pub(super) fn new(display_handle: DisplayHandle) -> Self {
        Self {
            inner: Default::default(),
            alive_tracker: Default::default(),
            display_handle,
        }
    }
}

impl<D> Dispatch2<WlDataSource, D> for DataSourceUserData
where
    D: DataDeviceHandler,
    D: 'static,
{
    fn request(
        &self,
        _state: &mut D,
        _client: &wayland_server::Client,
        _resource: &WlDataSource,
        request: wl_data_source::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut wayland_server::DataInit<'_, D>,
    ) {
        let mut data = self.inner.lock().unwrap();

        match request {
            wl_data_source::Request::Offer { mime_type } => {
                data.mime_types.push(mime_type);
            }
            wl_data_source::Request::SetActions { dnd_actions } => match dnd_actions {
                wayland_server::WEnum::Value(dnd_actions) => {
                    data.dnd_actions = DndAction::vec_from_wl(dnd_actions);
                }
                wayland_server::WEnum::Unknown(action) => {
                    error!("Unknown dnd_action: {:?}", action);
                }
            },
            wl_data_source::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, source: &WlDataSource) {
        self.alive_tracker.destroy_notify();

        // Remove the source from the used ones.
        let seat = match state
            .data_device_state()
            .used_sources
            .remove(source)
            .as_ref()
            .and_then(Seat::<D>::from_resource)
        {
            Some(seat) => seat,
            None => return,
        };

        let seat_data = seat
            .user_data()
            .get::<RefCell<SeatData<D::SelectionUserData>>>()
            .unwrap();

        // Ownership test under a SCOPED immutable borrow. The replacement hook below takes
        // `&mut D`; holding a `SeatData` borrow across it would panic for any handler that
        // reaches back into this seat's data.
        let owns_clipboard = matches!(
            seat_data.borrow().get_clipboard_selection(),
            Some(OfferReplySource::Client(SelectionSourceProvider::DataDevice(set_source)))
                if set_source == source
        );
        if !owns_clipboard {
            return;
        }

        // Ask the compositor for a replacement so the selection is swapped in ONE transition.
        // Clearing here and letting the compositor re-install afterwards would make clients
        // observe `wl_data_device.selection(nil)` followed by a fresh offer. The default impl
        // returns `None`, which reproduces the previous behaviour exactly.
        let replacement = state
            .selection_source_destroyed(SelectionTarget::Clipboard, &seat)
            .map(|(mime_types, user_data)| {
                OfferReplySource::Compositor(CompositorSelectionProvider {
                    ty: SelectionTarget::Clipboard,
                    mime_types,
                    user_data,
                })
            });

        seat_data
            .borrow_mut()
            .set_clipboard_selection::<D>(&self.display_handle, replacement);
    }
}

impl IsAlive for WlDataSource {
    #[inline]
    fn alive(&self) -> bool {
        let data: &DataSourceUserData = self.data().unwrap();
        data.alive_tracker.alive()
    }
}

impl Source for WlDataSource {
    fn metadata(&self) -> Option<SourceMetadata> {
        self.data::<DataSourceUserData>()
            .map(|data| data.inner.lock().unwrap().clone())
    }

    fn choose_action(&self, action: DndAction) {
        self.action(action.into());
    }

    fn send(&self, mime_type: &str, fd: OwnedFd) {
        debug!(?mime_type, "DnD transfer request");
        self.send(mime_type.to_owned(), fd.as_fd());
    }

    fn drop_performed(&self) {
        if self.version() >= wl_data_source::EVT_DND_DROP_PERFORMED_SINCE {
            self.dnd_drop_performed();
        }
    }

    fn cancel(&self) {
        self.cancelled();
    }

    fn finished(&self) {
        if self.version() >= wl_data_source::EVT_DND_FINISHED_SINCE {
            self.dnd_finished();
        }
    }
}

impl Source for WlSurface {
    fn is_client_local(&self, target: &dyn Any) -> bool {
        target
            .downcast_ref::<WlSurface>()
            .is_some_and(|target| target.id().same_client_as(&self.id()))
    }

    fn metadata(&self) -> Option<SourceMetadata> {
        None
    }

    fn choose_action(&self, action: DndAction) {
        let _ = action;
    }

    fn send(&self, mime_type: &str, fd: OwnedFd) {
        let _ = (mime_type, fd);
        unreachable!("Local dnd drops can't send");
    }

    fn drop_performed(&self) {}
    fn cancel(&self) {}
    fn finished(&self) {}
}

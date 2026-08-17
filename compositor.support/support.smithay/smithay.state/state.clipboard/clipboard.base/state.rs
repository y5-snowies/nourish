use std::os::fd::OwnedFd;

use compositor_support_smithay_state_clipboard_capture::capture::ClipboardCapture;
use compositor_support_smithay_state_clipboard_worker::worker::Worker;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::wayland::selection::data_device::DataDeviceState;

pub struct Clipboard {
    // Manages `wl_data_device_manager`. Handles copy/paste (clipboard) and drag-and-drop.
    pub data_device_state: DataDeviceState,
    // Snapshot of the current selection, so the clipboard survives the copying client
    // exiting. A wayland selection is a live object owned by that client, so without
    // this it dies with the app — see `clipboard.capture`.
    pub capture: ClipboardCapture,
    // The thread that actually reads the selection out of the copying client. Started on
    // the first copy rather than at boot: it needs a calloop `Ping` to wake the loop with,
    // and the factory that builds this has no loop handle. `None` until then, and also if
    // the thread could not be spawned — the clipboard then simply does not persist.
    pub worker: Option<Worker>,
    // Flavors to capture: recorded by `new_selection`, armed by the rim drain
    // (which owns the `loop_handle` the worker needs). Deferring is not just
    // convenience — `new_selection` fires BEFORE smithay installs the new
    // selection, so reading there would read the PREVIOUS clipboard.
    // The `usize` is the flavor's index in the client's advertised list: capture
    // order is ours, the offer we rebuild later is the client's.
    pub pending_capture: Option<(u64, Vec<(usize, String)>)>,
    // Reads of a compositor-owned selection: `send_selection` records the fd,
    // the drain hands it to the worker. `u64` is the capture generation the read
    // was answered against; `ClientId` is who asked, so the worker can judge a
    // client's pastes together.
    pub pending_sends: Vec<(u64, ClientId, String, OwnedFd)>,
    // Deferred `set_data_device_focus` (needs DataDeviceHandler — downstream);
    // recorded by `focus_changed`, applied by the rim drain. The inner
    // `Option<Client>` is the focused client (None == clear focus).
    pub pending_focus: Option<Option<Client>>,
}
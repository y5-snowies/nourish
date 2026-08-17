use smithay::reexports::wayland_server::{DisplayHandle, GlobalDispatch};
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::WlDataDeviceManager;
use smithay::wayland::GlobalData;
use smithay::wayland::selection::data_device::{DataDeviceHandler, DataDeviceState};
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;
use compositor_support_smithay_state_clipboard_base::state::Clipboard;

pub fn new<I: DispatchWire>(display_handle: &DisplayHandle) -> Clipboard where
    I: GlobalDispatch<WlDataDeviceManager, GlobalData> + 'static,
    I: DataDeviceHandler,{
    // Initialize Clipboard and Drag-and-Drop functionality.
    // Side-effect: Interacts with `calloop` heavily when users highlight text or drag files,
    // negotiating MIME types between two separate Wayland clients asynchronously.
    let data_device_state = DataDeviceState::new::<I>(&display_handle);

    return Clipboard{
        data_device_state,
        capture: Default::default(),
        // Started lazily by the drain on the first copy — it needs a loop handle.
        worker: None,
        pending_capture: None,
        pending_sends: vec![],
        pending_focus: None,
    }

}

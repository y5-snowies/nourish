use compositor_y5_lock_tty_interface::interface::VtSwitcher;
use compositor_support_library_pam_worker_base::PamWorker;
use smithay::reexports::calloop::RegistrationToken;
use compositor_support_bevy_core_compositor_base::BevyHandle;
use compositor_monitor_compositor_iced_base::{HandleId, IcedHandle};

pub struct LockState {
    pub active: Option<LockActiveState>,
    pub tty: Option<VtSwitcher>,
    pub pam: Option<(PamWorker, RegistrationToken)>,
}

#[derive(Clone)]
pub struct LockActiveState {
    pub bevy: Option<BevyHandle<compositor_background_three_lock_scene::MorphScene>>,
    pub surface: Vec<HandleId>,
    pub capture: LockActiveCapture,
    pub surface_input: Option<IcedHandle<compositor_y5_lock_interface_surface::view::LockSurface>>,
    /// Set once the morph fold has been dispatched — at the `pending`→done
    /// handoff, when the originating session scene is dropped. Gates the fold
    /// to fire exactly once (the snapshot plane is already up before this).
    pub fold_started: bool,
}

#[derive(Clone)]
pub enum LockActiveCapture {
    None,
    Capture(compositor_y5_graphic_capture_registry::CaptureHandle),
    Snapshot(compositor_y5_graphic_capture_registry::SnapshotHandle),
}

impl LockState {
    pub fn new() -> Self {
        let tty = match VtSwitcher::new() {
            Ok(vtt) => Some(vtt),
            Err(err) => {
                error!("VT Switcher creation failed: {:?}", err);
                None
            }
        };

        return Self { pam: None, active: None, tty };
    }
}

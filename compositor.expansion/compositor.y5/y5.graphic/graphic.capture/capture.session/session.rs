//! The capture phase state machine and the state stored on the compositor.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use smithay::utils::{Logical, Physical, Point, Rectangle};
use uuid::Uuid;
use compositor_y5_graphic_capture_encode::{AsyncReadback, Frame, ReencodeJob};
use compositor_y5_graphic_capture_registry::CaptureHandle;
use compositor_y5_graphic_capture_vaapi::CaptureEncoder;
use compositor_monitor_compositor_iced_base::HandleId;

use crate::message::{CaptureMedia, TargetKind};

/// What is being captured, with its geometry, in its natural coordinate space.
#[derive(Clone, Debug)]
pub enum CaptureTarget {
    /// The set of selected windows (by uuid). The captured rect is the union
    /// of their y5-world geometries, recomputed on move/resize.
    Windows(Vec<Uuid>),
    /// A rectangle anchored in y5-world (pans/zooms with the camera).
    WorldRegion(Rectangle<i32, Logical>),
    /// A rectangle fixed in screen/output pixels (camera-independent).
    ScreenRegion(Rectangle<i32, Physical>),
    /// The whole output.
    FullScreen,
}

impl CaptureTarget {
    pub fn kind(&self) -> TargetKind {
        match self {
            CaptureTarget::Windows(_) => TargetKind::Windows,
            CaptureTarget::WorldRegion(_) => TargetKind::WorldRegion,
            CaptureTarget::ScreenRegion(_) => TargetKind::ScreenRegion,
            CaptureTarget::FullScreen => TargetKind::FullScreen,
        }
    }
}

/// Active capture: the registry is feeding `capture`; the border/dim/stop
/// indicators are up.
///
/// The video keep-alive (5-minute prompt + 30-second grace) is deadline-based
/// and serviced from the per-frame hook (which has a renderer to spawn the
/// dialog), not a calloop timer.
pub struct ActiveState {
    pub media: CaptureMedia,
    pub target: CaptureTarget,
    pub capture: CaptureHandle,
    /// Transparent-background option: when true the per-element capture omits
    /// the parallax/iced backdrop and emits transparent where no window is.
    pub no_background: bool,
    /// Hardware video encoder (video only): NVENC (readback-fed) or VAAPI
    /// (zero-copy dmabuf), selected by `Y5_CAPTURE_ENCODER`. `None` if init
    /// failed (no software fallback).
    pub encoder: Option<CaptureEncoder>,
    /// GPU→CPU readback feeding the NVENC encoder (`None` for the VAAPI path,
    /// which reads the dmabuf directly).
    pub readback: Option<AsyncReadback>,
    /// y5-world top-left of the (fixed) capture region — windows are rendered
    /// relative to this in the per-element capture so the encoder's dmabuf
    /// stays a constant size.
    pub region_origin: Point<i32, Logical>,
    /// Last time a frame was encoded (frame-rate throttle).
    pub last_frame: Option<Instant>,
    /// Wall-clock anchor for the first encoded video frame. Frame PTS are derived
    /// from `now - video_start` (not a counter), so the recorded timeline matches
    /// real time regardless of the achieved capture rate vs. the configured max.
    pub video_start: Option<Instant>,
    /// Frames waited for a screenshot before reading back (lets the capture tap
    /// fill the entry). `None` for video; `Some(n)` counts up for screenshots.
    pub shot_wait: Option<u8>,
    /// Last physical crop pushed to the registry + indicators; used to skip
    /// redundant per-frame updates (which would re-render the full-screen
    /// border/dim every frame — the region-video lag).
    pub last_crop: Option<Rectangle<i32, Physical>>,
    /// Anchor for the 5-minute keep-alive countdown; reset on Continue.
    pub keepalive_anchor: Instant,
    /// When `Some`, the continue dialog is up and the capture auto-stops at
    /// this instant unless the user continues.
    pub dialog_deadline: Option<Instant>,
}

/// The finished, not-yet-saved capture artifact held during the Saving phase.
pub enum PendingSave {
    /// A screenshot frame (read back at Stop).
    Image(Frame),
    /// A finished video at this temp mp4 path.
    Video(PathBuf),
}

pub enum CapturePhase {
    Idle,
    /// The setup overlay is up and owns input. The overlay UI
    /// (`SetupOverlay`) is authoritative for the chosen kind/media/draft rect;
    /// `begin_active` reads them from the instance on Confirm.
    Setup,
    Active(ActiveState),
    /// Capture stopped; the Save dialog (Save / Save As / Discard) is up.
    Saving {
        media: CaptureMedia,
        pending: PendingSave,
        /// When `Some`, a background "Save As" portal call is in flight; the
        /// per-frame hook drains its result (`Some(path)` = save there,
        /// `None` = portal failed/cancelled → keep the dialog up).
        saveas: Option<Receiver<Option<PathBuf>>>,
    },
    /// Optimized (software) re-encode in progress for the manual "Optimized
    /// encoding" path: the progress dialog is up; the per-frame hook polls `job`
    /// and updates the bar. On success `output` is moved to `target`; on failure
    /// we revert to the Save dialog so the `lossless` temp can still be saved.
    Encoding {
        job: ReencodeJob,
        target: PathBuf,
        output: PathBuf,
        lossless: PathBuf,
    },
}

/// All capture state, stored on the compositor `State` as `state.inner.kernel.get(&compositor_orchestration_driver_capture_base::base::CAPTURE)`.
pub struct CaptureState {
    pub phase: CapturePhase,
    /// Set by the Super+S keybinding; drained by the per-frame hook (which has
    /// a renderer) to spawn the setup overlay.
    pub pending_setup: bool,
    // iced handle ids for the live overlay elements (None when absent):
    pub setup_id: Option<HandleId>,
    pub border_id: Option<HandleId>,
    pub dim_id: Option<HandleId>,
    /// One Stop button per physical output — each bound (output-affinity) to its
    /// monitor and anchored to that monitor's top-right, so the control is
    /// reachable on every screen while recording.
    pub stop_hud_ids: Vec<HandleId>,
    pub continue_dialog_id: Option<HandleId>,
    pub save_dialog_id: Option<HandleId>,
    /// Window uuids the render path must keep rendering (and giving frame
    /// callbacks / presentation feedback) regardless of on-screen visibility.
    pub force_set: HashSet<Uuid>,
    /// Cached y5-world bbox of a `Windows` target, recomputed only on window
    /// move/resize/destroy — not per frame.
    pub windows_bbox: Option<Rectangle<i32, Logical>>,
    /// Window selection snapshotted when setup started (Super+S). The capture
    /// uses THIS, not the live canvas selection, so interacting with the setup
    /// overlay (e.g. switching media) can't change what gets captured.
    pub setup_selection: Vec<Uuid>,
}

impl CaptureState {
    pub fn idle() -> Self {
        Self {
            phase: CapturePhase::Idle,
            pending_setup: false,
            setup_id: None,
            border_id: None,
            dim_id: None,
            stop_hud_ids: Vec::new(),
            continue_dialog_id: None,
            save_dialog_id: None,
            force_set: HashSet::new(),
            windows_bbox: None,
            setup_selection: Vec::new(),
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.phase, CapturePhase::Idle)
    }

    pub fn is_setup(&self) -> bool {
        matches!(self.phase, CapturePhase::Setup)
    }

    pub fn is_active(&self) -> bool {
        matches!(self.phase, CapturePhase::Active(_))
    }

    pub fn is_saving(&self) -> bool {
        matches!(self.phase, CapturePhase::Saving { .. })
    }

    pub fn is_encoding(&self) -> bool {
        matches!(self.phase, CapturePhase::Encoding { .. })
    }
}

impl Default for CaptureState {
    fn default() -> Self {
        Self::idle()
    }
}

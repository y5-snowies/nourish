use compositor_monitor_compositor_iced_base::IcedHandle;
use compositor_support_bevy_core_compositor_base::BevyHandle;
use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use compositor_y5_graphic_capture_registry::{CaptureHandle, SnapshotHandle};
use compositor_y5_picker_surface_view::PickerSurface;
use compositor_y5_picker_three_scene::PickerScene;
use std::collections::HashMap;
use std::time::Instant;

/// Cube-sphere cell layout: 6 faces × `CELLS_PER_FACE²` = 54 cells (fixed grid;
/// worlds created lazily as empty cells are entered).
pub const CELLS_PER_FACE: usize = 3;
pub const CELL_COUNT: usize = 6 * CELLS_PER_FACE * CELLS_PER_FACE;

/// The world-selection state slot (token lives here, cycle-free).
pub static PICKER: Token<PickerState> = Token::new();
pub static PICKER_MUT: TokenMut<PickerState> = TokenMut::new(&PICKER);

/// Lives in the PICKER overlay world (persists across opens). `active` is `Some`
/// only while on screen.
pub struct PickerState {
    pub active: Option<PickerActive>,
    /// PERSISTENT cell → world map (None = empty; Enter lazily creates). Len `CELL_COUNT`.
    pub cell_worlds: Vec<Option<uuid::Uuid>>,
    /// Most-recent frozen thumbnail per world id (captured on-leave).
    pub thumbnails: HashMap<uuid::Uuid, SnapshotHandle>,
    /// User-given name per world id (shown + editable in the details panel).
    pub world_names: HashMap<uuid::Uuid, String>,
    /// In-flight capture arming for the deferred open (see picker.interface).
    pub arming: Option<Arming>,
    /// An open is pending: when the outgoing world's fade-to-black started. The
    /// switch waits for it, so the transition meets on an opaque frame.
    pub fade_out: Option<Instant>,
}

pub struct Arming {
    pub origin: uuid::Uuid,
    pub capture: CaptureHandle,
    /// Frames to let the capture fill before snapshotting.
    pub countdown: u8,
}

pub struct PickerActive {
    /// World active when the picker opened — where `cancel` returns.
    pub origin: uuid::Uuid,
    /// Focused cell (arrow keys / click; outlined + "+" in the scene).
    pub selected: Option<usize>,
    /// Latest pointer position in output px (picker tracks the cursor itself).
    pub pointer: (f64, f64),
    /// `Some(press_pos)` while a left-drag is in progress (drag starts anywhere).
    pub drag: Option<(f64, f64)>,
    /// Sphere orientation quaternion (xyzw) + the target it animates toward.
    pub orientation: [f32; 4],
    pub target: [f32; 4],
    /// Per-frame trackball increment carried as drag-release momentum (decays).
    pub spin: [f32; 4],
    /// Camera zoom (scroll); distance = base / zoom.
    pub zoom: f32,
    /// The bevy sphere instance (picker world's registry).
    pub bevy: Option<BevyHandle<PickerScene>>,
    /// The bottom-right details panel (iced, session registry).
    pub surface: Option<IcedHandle<PickerSurface>>,
    /// When the picker opened — drives the entry transition.
    pub time: Instant,
    /// When the orientation was last advanced — the dt source for momentum and
    /// the re-face glide, so both run in wall time rather than in frames.
    pub step: Instant,
}

impl PickerState {
    pub fn new() -> Self {
        Self {
            active: None, cell_worlds: vec![None; CELL_COUNT], thumbnails: HashMap::new(),
            world_names: HashMap::new(), arming: None, fade_out: None,
        }
    }

    /// Ensure `world` occupies a cell, returning its cell index. Assigns the
    /// first empty cell if it isn't mapped yet.
    pub fn ensure_cell(&mut self, world: uuid::Uuid) -> usize {
        if let Some(i) = self.cell_worlds.iter().position(|c| *c == Some(world)) {
            return i;
        }
        let i = self.cell_worlds.iter().position(|c| c.is_none()).unwrap_or(0);
        self.cell_worlds[i] = Some(world);
        i
    }
}

impl Default for PickerState {
    fn default() -> Self {
        Self::new()
    }
}

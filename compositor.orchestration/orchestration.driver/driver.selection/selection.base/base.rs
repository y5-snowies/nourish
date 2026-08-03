use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use compositor_monitor_compositor_iced_base::HandleId;
use smithay::utils::{Physical, Point};

/// Selection-overlay driver data: the live iced toolbar instance (created when
/// the selection becomes non-empty, destroyed when it empties) plus the
/// last-seen selection count used to gate redundant UI dispatches. The handle
/// is shared between the render-path reconciler (create/destroy/count) and the
/// `SelectSystem`-driven reposition (see `compositor_y5_select_overlay_system`).
pub struct SelectionOverlayState {
    /// The live toolbar instance, if one is currently shown.
    pub handle: Option<HandleId>,
    /// Companion hover-tooltip surface (separate texture, floats above the bar,
    /// click-through). Lives alongside `handle`.
    pub tip_handle: Option<HandleId>,
    /// Last tooltip content pushed to the tip surface — gates redundant
    /// re-renders while the same button stays hovered.
    pub last_tip: Option<(String, bool, bool)>,
    /// Last selection size pushed to the UI (avoids redundant dispatches).
    pub count: i32,
    /// Last camera zoom the world toolbar was counter-scaled for (NaN = unset).
    pub prev_zoom: f64,
}

impl Default for SelectionOverlayState {
    fn default() -> Self {
        Self {
            handle: None,
            tip_handle: None,
            last_tip: None,
            count: 0,
            prev_zoom: f64::NAN,
        }
    }
}

pub static SELECTION_OVERLAY: Token<SelectionOverlayState> = Token::new();
pub static SELECTION_OVERLAY_MUT: TokenMut<SelectionOverlayState> =
    TokenMut::new(&SELECTION_OVERLAY);

/// One-shot "re-anchor the toolbar to the cursor" flag. Set by
/// `compositor_y5_select_overlay_system` when it receives a selection-change
/// event; consumed (read + cleared) by the render-path reconciler, which has
/// the seat to read the live cursor. Lives in the spatial world's storage
/// (registered by the overlay system, resolved via the spawn-target accessor).
pub static SELECTION_REANCHOR: Token<bool> = Token::new();
pub static SELECTION_REANCHOR_MUT: TokenMut<bool> = TokenMut::new(&SELECTION_REANCHOR);

/// Where the selection toolbar is placed. Compile-time knob, shared by the
/// reconciler (placement at create) and the reposition system.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Fixed at the bottom-center of the screen (zoom-independent).
    ScreenBottomCenter,
    /// World-space, centered just below the cursor, above all windows. Scales
    /// with camera zoom and re-anchors to the cursor on each selection change.
    WorldAtCursor,
}

/// The active placement. Flip this constant to switch modes.
pub const SELECTION_OVERLAY_PLACEMENT: Placement = Placement::WorldAtCursor;

/// On-screen toolbar size in physical pixels — the size it keeps on screen at
/// ANY zoom (WorldAtCursor counter-scales the world surface to hold this).
pub const BAR_W: i32 = 440;
pub const BAR_H: i32 = 120;
/// On-screen size of the hover-tooltip surface (a separate texture floating
/// above the bar). Fixed screen-physical px; it's a screen-space passthrough
/// surface, so it doesn't need the world counter-scale the bar uses.
pub const TIP_W: i32 = 260;
pub const TIP_H: i32 = 56;
/// Gap between the tooltip's bottom and the bar's top, in screen px.
pub const TIP_GAP: i32 = 8;
/// Gap below the screen bottom (ScreenBottomCenter).
pub const SCREEN_BOTTOM_MARGIN: i32 = 100;
/// On-screen gap below the cursor, in physical px (WorldAtCursor).
pub const CURSOR_DY: f64 = 12.0;
/// Lower bound on the zoom the world half-extents are divided by, so a camera
/// parked at a near-zero zoom can't send the toolbar off to infinity.
pub const MIN_ZOOM: f64 = 0.15;

/// The toolbar's half-width/half-height in WORLD units at a given zoom.
///
/// The surface is zoom-LOCKED (`set_zoom_locked_by_id`): `BAR_W`×`BAR_H` is a
/// count of SCREEN pixels at every zoom, and the texture is rasterized at
/// exactly that size once. So only the world-space EXTENT still varies with
/// zoom, and only for placing the thing — nothing is ever re-rasterized.
///
/// It used to be the other way round: a world footprint of `BAR/zoom` with an
/// iced factor of `1/zoom`, which held the on-screen size constant by shrinking
/// the buffer — at 4× zoom a quarter-resolution toolbar upscaled 4× on screen.
pub fn world_half(zoom: f64) -> (f64, f64) {
    let z = zoom.max(MIN_ZOOM);
    ((BAR_W as f64) / z / 2.0, (BAR_H as f64) / z / 2.0)
}

/// World-physical top-left so the (on-screen constant `BAR_W`-wide) toolbar is
/// centered horizontally on, and just below, the cursor. `cursor` is the live
/// world-logical cursor (seat `current_location`); `scale` the output scale;
/// world iced stores location in `logical × scale` units (like placeholders).
pub fn world_loc_under_cursor(cursor: (f64, f64), scale: f64, zoom: f64) -> Point<i32, Physical> {
    let (half_w, _) = world_half(zoom);
    let z = zoom.max(MIN_ZOOM);
    Point::from((
        (cursor.0 * scale - half_w).round() as i32,
        (cursor.1 * scale + CURSOR_DY / z).round() as i32,
    ))
}

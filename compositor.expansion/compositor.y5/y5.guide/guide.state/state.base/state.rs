//! Guide-popup driver state: the empty-canvas context menu and the help panel.
//!
//! Both surfaces are DESIRED here and RECONCILED by `interface.create` on the
//! render path (creating an iced surface needs the `GlesRenderer`). So any code
//! anywhere may open or dismiss a popup by writing this slot — which is what
//! lets the "any key / any outside click closes it" rule live in the rim input
//! handlers without them ever touching the renderer.

use compositor_monitor_compositor_iced_base::HandleId;
use compositor_support_system_storage_token_base::base::{Token, TokenMut};
use smithay::utils::{Physical, Point, Size};

/// On-screen size of the icon menu, physical px: two icon cells and nothing
/// else. The hovered entry's shortcut floats BELOW in its own tooltip texture
/// (`menu.tip`), like the selection toolbar's — inside the menu it would have to
/// reserve room it does not need, and re-laying the counter-scaled world surface
/// on every hover made the menu visibly lag the pointer.
pub const MENU_W: i32 = 250;
pub const MENU_H: i32 = 48;
/// Supersample factor for the menu's texture: the dmabuf holds `MENU × SS`
/// pixels and is downscaled to `MENU` on screen. Material glyphs at this size
/// are only ~16px tall, and rasterizing them 1:1 left the icons visibly chewed
/// while the selection toolbar — whose icons are half again as large — looked
/// fine. Costs one 500×96 buffer instead of 250×48, allocated once.
pub const MENU_SUPERSAMPLE: f32 = 2.0;

/// The menu's dmabuf size: the on-screen size times the supersample factor.
pub fn menu_buffer() -> Size<i32, Physical> {
    Size::from((
        ((MENU_W as f32) * MENU_SUPERSAMPLE).round() as i32,
        ((MENU_H as f32) * MENU_SUPERSAMPLE).round() as i32,
    ))
}

/// On-screen size of the hover tooltip and its gap below the menu, physical px.
pub const TIP_W: i32 = 200;
pub const TIP_H: i32 = 34;
pub const TIP_GAP: i32 = 6;
/// On-screen size of the help panel, physical px.
pub const HELP_W: i32 = 380;
pub const HELP_H: i32 = 258;
/// Gap below the cursor the menu is anchored at, physical px.
pub const CURSOR_DY: f64 = 10.0;
/// Lower bound on the zoom the anchor offset is divided by, so a camera parked
/// at a near-zero zoom can't send the menu off to infinity.
pub const MIN_ZOOM: f64 = 0.15;
/// Double-click window (ms) and max travel (world px) that still opens the menu.
pub const DOUBLE_CLICK_MS: u32 = 400;
pub const DOUBLE_CLICK_SLOP: f64 = 6.0;

#[derive(Default)]
pub struct GuideState {
    /// Menu anchor in world-logical coords — `Some` = the menu should be shown.
    pub menu_at: Option<(f64, f64)>,
    /// The live menu surface, once the reconciler has built it.
    pub menu: Option<HandleId>,
    /// Its companion tooltip surface — separate texture, click-through, hidden
    /// until something is hovered. Lives and dies with `menu`.
    pub tip: Option<HandleId>,
    /// Last text pushed into the tip; gates redundant re-renders while the same
    /// entry stays hovered.
    pub last_tip: Option<String>,
    /// Camera zoom the menu's anchor offset was last derived for (0 = unset).
    pub menu_zoom: f64,
    /// The help panel should be shown / is shown.
    pub help_open: bool,
    pub help: Option<HandleId>,
    /// Last empty-canvas press (time ms, world x, y) — the double-click state.
    pub last_click: Option<(u32, f64, f64)>,
}

impl GuideState {
    /// True while either popup is up or requested — the gate the input rim uses
    /// to decide whether a press/key has a popup to dismiss.
    pub fn showing(&self) -> bool {
        self.menu_at.is_some() || self.help_open
    }
}

pub static GUIDE: Token<GuideState> = Token::new();
pub static GUIDE_MUT: TokenMut<GuideState> = TokenMut::new(&GUIDE);

/// World-physical top-left placing the menu centred on, and just below, the
/// anchor. World iced stores location in `logical × scale` units.
///
/// The surface is zoom-LOCKED, so `MENU_W` is a count of SCREEN pixels at any
/// zoom — which makes its half-width in WORLD units `MENU_W/2/zoom`. That is why
/// this still depends on zoom even though the size no longer does, and why the
/// caller re-derives it whenever the camera zoom moves.
pub fn world_loc(anchor: (f64, f64), scale: f64, zoom: f64) -> Point<i32, Physical> {
    let z = zoom.max(MIN_ZOOM);
    Point::from((
        (anchor.0 * scale - (MENU_W as f64) / z / 2.0).round() as i32,
        (anchor.1 * scale + CURSOR_DY / z).round() as i32,
    ))
}

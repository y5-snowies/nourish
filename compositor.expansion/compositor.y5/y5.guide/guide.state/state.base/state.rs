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
pub const MENU_W: i32 = 180;
pub const MENU_H: i32 = 34;
/// On-screen size of the hover tooltip and its gap below the menu, physical px.
pub const TIP_W: i32 = 180;
pub const TIP_H: i32 = 30;
pub const TIP_GAP: i32 = 6;
/// On-screen size of the help panel, physical px.
pub const HELP_W: i32 = 380;
pub const HELP_H: i32 = 258;
/// Gap below the cursor the menu is anchored at, physical px.
pub const CURSOR_DY: f64 = 10.0;
/// Lower bound on the zoom used for counter-scaling, so the world dmabuf
/// (`MENU / zoom`) can't explode past GPU limits when zoomed far out.
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
    /// Camera zoom the menu was last counter-scaled for (NaN-free: 0 = unset).
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

/// World footprint that renders to `w`×`h` ON SCREEN at the given zoom (a World
/// item's screen size = world size × zoom), so the menu keeps one size however
/// far the canvas is zoomed. Mirrors the selection toolbar's counter-scale.
pub fn world_size(w: i32, h: i32, zoom: f64) -> Size<i32, Physical> {
    let z = zoom.max(MIN_ZOOM);
    Size::from((
        ((w as f64) / z).round().max(1.0) as i32,
        ((h as f64) / z).round().max(1.0) as i32,
    ))
}

/// iced scale factor for that counter-scaled surface, so the content lays out at
/// the native size and fills the (larger, when zoomed out) dmabuf.
pub fn world_scale_factor(zoom: f64) -> f32 {
    (1.0 / zoom.max(MIN_ZOOM)) as f32
}

/// World-physical top-left placing the menu centred on, and just below, the
/// anchor. World iced stores location in `logical × scale` units.
pub fn world_loc(anchor: (f64, f64), scale: f64, zoom: f64) -> Point<i32, Physical> {
    let size = world_size(MENU_W, MENU_H, zoom);
    let z = zoom.max(MIN_ZOOM);
    Point::from((
        (anchor.0 * scale - (size.w as f64) / 2.0).round() as i32,
        (anchor.1 * scale + CURSOR_DY / z).round() as i32,
    ))
}

//! The empty-canvas context menu: Settings and Help as thin white rings in a
//! slim pill. Hovering brightens the ring and floats that entry's shortcut in a
//! separate tooltip surface below (`menu.tip`). The hints come from
//! `menu.create`, which reads the live keybindings; this crate only republishes
//! the hovered one through `IcedSnapshot`.

use compositor_monitor_selection_font_base::font::MATERIAL_FAMILY;
use compositor_monitor_selection_font_base::font_map;
use compositor_support_iced_core_engine_base::{IcedSnapshot, IcedUi, Renderer};
use iced_core::{Background, Border, Color, Element, Length, Padding, Theme};
use iced_widget::{button, container, mouse_area, row, text};

/// Glyph size / ring diameter; the pill height is `MENU_H`, its radius silhouette.
const ICON: f32 = 16.0;
const CELL: f32 = 32.0;

/// Which entry the pointer is over (drives the tooltip).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuideItem {
    Settings,
    Help,
}

/// `Hover` never leaves the surface; the other two are the actionable clicks.
#[derive(Clone, Debug)]
pub enum GuideMessage {
    OpenSettings,
    OpenHelp,
    Hover(Option<GuideItem>),
}

#[derive(Default)]
pub struct GuideMenu {
    pub hovered: Option<GuideItem>,
    /// Tooltip text for each entry — the bare shortcut, nothing else.
    pub settings_hint: String,
    pub help_hint: String,
}

fn icon<'a>(g: &'static str, m: GuideMessage, item: GuideItem, on: bool) -> Element<'a, GuideMessage, Theme, Renderer> {
    let tint = if on { Color::WHITE } else { Color::from_rgba(1.0, 1.0, 1.0, 0.72) };
    let glyph = text(g).font(MATERIAL_FAMILY).size(ICON).center().style(move |_t: &Theme| text::Style { color: Some(tint) });
    let ring = Border { color: Color::from_rgba(1.0, 1.0, 1.0, if on { 0.85 } else { 0.35 }), width: 1.0, radius: (CELL / 2.0).into() };
    let cell = button(container(glyph).center_x(Length::Fill).center_y(Length::Fill).height(Length::Fixed(CELL)))
        .width(Length::Fixed(CELL))
        .padding(0)
        .on_press(m)
        .style(move |_t: &Theme, _s| button::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, if on { 0.10 } else { 0.0 }))),
            text_color: Color::WHITE,
            border: ring,
            ..Default::default()
        });
    mouse_area(cell).on_enter(GuideMessage::Hover(Some(item))).on_exit(GuideMessage::Hover(None)).into()
}

impl IcedUi for GuideMenu {
    type Message = GuideMessage;

    fn update(&mut self, message: Self::Message) {
        if let GuideMessage::Hover(h) = message { self.hovered = h; }
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let h = self.hovered;
        let content = row![
            icon(font_map::Settings, GuideMessage::OpenSettings, GuideItem::Settings, h == Some(GuideItem::Settings)),
            icon(font_map::QuestionMark, GuideMessage::OpenHelp, GuideItem::Help, h == Some(GuideItem::Help)),
        ]
        .spacing(12);
        container(content)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding { top: 8.0, bottom: 8.0, left: 20.0, right: 20.0 })
            .style(|_t| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.06, 0.07, 0.10, 0.90))),
                border: Border { color: Color::from_rgba(1.0, 1.0, 1.0, 0.14), width: 1.0, radius: 14.0.into() },
                ..Default::default()
            })
            .into()
    }
}

/// The hovered entry's hint, read once per frame by the tooltip driver. A tip is
/// frame-latent anyway — hover, next frame it shows — so a one-frame-old
/// snapshot costs nothing.
impl IcedSnapshot for GuideMenu {
    type Snapshot = Option<String>;
    fn snapshot(&self) -> Self::Snapshot {
        match self.hovered? {
            GuideItem::Settings => Some(self.settings_hint.clone()),
            GuideItem::Help => Some(self.help_hint.clone()),
        }
    }
}

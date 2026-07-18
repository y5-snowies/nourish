//! The sticky touch pane: a left-centre vertical strip of large Material-icon
//! buttons that toggle the four touch tool-modes (Touch / Pointer / Hand /
//! Select) and open Overview / World Picker. Taps emit `TouchPaneMessage`s; the
//! compositor pushes `SetActive` back to reflect the live mode (highlighted).
//! Screen-space, so touch taps reach it through the emulated-pointer path.
use iced_core::{Background, Border, Color, Element, Length, Theme};
use iced_widget::{button, column, container, text, Space};
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use compositor_monitor_selection_font_base::font::MATERIAL_FAMILY;
use compositor_monitor_selection_font_base::font_map;

/// Icon glyph size and the square tap-cell height (large for touch).
const ICON: f32 = 34.0;
const CELL: f32 = 80.0;

/// The pane's copy of the touch tool-mode (mapped to/from the seat's `TouchMode`
/// in the handle crate, so this view crate stays free of orchestration deps).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PaneMode {
    #[default]
    Touch,
    Pointer,
    Hand,
    Select,
}

/// Messages between the pane and the compositor. `SetMode`/`Open*` are surface →
/// compositor (button taps); `SetActive` is compositor → surface (reflect mode).
#[derive(Clone, Debug)]
pub enum TouchPaneMessage {
    SetMode(PaneMode),
    OpenOverview,
    OpenWorldPicker,
    /// Open the app launcher (also auto-opens the OSK for search).
    OpenLauncher,
    /// Open the on-screen keyboard (pinned).
    OpenOsk,
    /// Dismiss the pane.
    Close,
    SetActive(PaneMode),
}

#[derive(Default)]
pub struct TouchPane {
    pub active: PaneMode,
}

impl TouchPane {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A large Material-icon button centred in a square tap-cell.
fn icon_button<'a>(
    glyph: &'static str,
    msg: TouchPaneMessage,
    on: bool,
) -> Element<'a, TouchPaneMessage, Theme, Renderer> {
    let label = text(glyph)
        .font(MATERIAL_FAMILY)
        .size(ICON)
        .center()
        .style(move |_t: &Theme| text::Style {
            color: Some(if on {
                Color::WHITE
            } else {
                Color::from_rgba(0.85, 0.9, 1.0, 0.8)
            }),
        });
    button(
        container(label)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fixed(CELL)),
    )
    .width(Length::Fill)
    .padding(0)
    .on_press(msg)
    .style(move |_t: &Theme, _s| button::Style {
        background: Some(Background::Color(if on {
            Color::from_rgba(0.30, 0.55, 1.0, 0.85)
        } else {
            Color::from_rgba(1.0, 1.0, 1.0, 0.07)
        })),
        text_color: Color::WHITE,
        border: Border { radius: 12.0.into(), ..Default::default() },
        ..Default::default()
    })
    .into()
}

impl IcedUi for TouchPane {
    type Message = TouchPaneMessage;

    fn update(&mut self, message: Self::Message) {
        if let TouchPaneMessage::SetActive(a) = message {
            self.active = a;
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let a = self.active;
        let tool = |glyph, mode| icon_button(glyph, TouchPaneMessage::SetMode(mode), mode == a);
        let content = column![
            tool(font_map::TouchApp, PaneMode::Touch),
            tool(font_map::Mouse, PaneMode::Pointer),
            tool(font_map::PanTool, PaneMode::Hand),
            tool(font_map::HighlightAlt, PaneMode::Select),
            Space::new().height(12),
            icon_button(font_map::Apps, TouchPaneMessage::OpenLauncher, false),
            icon_button(font_map::GridView, TouchPaneMessage::OpenOverview, false),
            icon_button(font_map::Public, TouchPaneMessage::OpenWorldPicker, false),
            icon_button(font_map::Keyboard, TouchPaneMessage::OpenOsk, false),
            Space::new().height(12),
            icon_button(font_map::Close, TouchPaneMessage::Close, false),
        ]
        .spacing(8)
        .width(Length::Fill);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(12)
            .style(|_t| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.05, 0.06, 0.09, 0.88))),
                border: Border { radius: 16.0.into(), ..Default::default() },
                ..Default::default()
            })
            .into()
    }
}

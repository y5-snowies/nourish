//! The help panel: a screen-space card listing the few shortcuts a first-time
//! user needs, each with the key it is CURRENTLY bound to (the rows are built
//! from the live registries by `help.row`, not written down here).
//!
//! Deliberately short. It is the answer to "I see a blank canvas, now what", not
//! a reference — the full, rebindable list is the settings Keys tab. Gestures
//! that differ only by one held modifier share a row, with the extra modifier as
//! a second, dimmer pill, so the list reads as five ideas rather than nine keys.

use compositor_monitor_selection_font_base::font::MATERIAL_FAMILY;
use compositor_monitor_selection_font_base::font_map;
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use compositor_y5_guide_menu_view::GuideMessage;
use iced_core::{alignment, Background, Border, Color, Element, Length, Padding, Theme};
use iced_widget::{column, container, row, text, Space};

/// One line: what it does, the combo it is bound to right now, and — for a
/// gesture with a modifier variant — the extra modifier that switches it.
pub struct HelpRow {
    pub label: String,
    pub combo: String,
    pub extra: Option<String>,
}

#[derive(Default)]
pub struct HelpPanel {
    pub rows: Vec<HelpRow>,
}

fn dim(v: f32) -> Color {
    Color::from_rgba(0.84, 0.88, 0.96, v)
}

/// A key pill. The variant one is dimmer — it is a qualifier, not a shortcut.
fn pill(label: &str, strong: bool) -> Element<'_, GuideMessage, Theme, Renderer> {
    let fg = if strong { Color::WHITE } else { dim(0.62) };
    container(text(label).size(12).style(move |_t: &Theme| text::Style { color: Some(fg) }))
        .padding(Padding { top: 2.0, bottom: 2.0, left: 7.0, right: 7.0 })
        .style(move |_t| container::Style {
            background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, if strong { 0.09 } else { 0.05 }))),
            border: Border { color: Color::from_rgba(1.0, 1.0, 1.0, 0.10), width: 1.0, radius: 5.0.into() },
            ..Default::default()
        })
        .into()
}

fn line(r: &HelpRow) -> Element<'_, GuideMessage, Theme, Renderer> {
    let mut keys = row![pill(&r.combo, true)].spacing(5).align_y(alignment::Vertical::Center);
    if let Some(extra) = &r.extra {
        keys = keys.push(pill(extra, false));
    }
    row![
        text(r.label.as_str()).size(13).style(|_t: &Theme| text::Style { color: Some(dim(0.92)) }),
        Space::new().width(Length::Fill),
        keys,
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center)
    .into()
}

impl IcedUi for HelpPanel {
    type Message = GuideMessage;

    /// Read-only: dismissed by the input rim (any key, any click outside), so no
    /// message of its own ever reaches it.
    fn update(&mut self, _message: Self::Message) {}

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let title = row![
            text(font_map::QuestionMark).font(MATERIAL_FAMILY).size(16).style(|_t: &Theme| text::Style { color: Some(dim(0.75)) }),
            text("Help").size(14).style(|_t: &Theme| text::Style { color: Some(Color::WHITE) }),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

        let mut body = column![title].spacing(11);
        for r in &self.rows {
            body = body.push(line(r));
        }

        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding { top: 22.0, bottom: 22.0, left: 26.0, right: 26.0 })
            .style(|_t| container::Style {
                background: Some(Background::Color(Color::from_rgb(0.06, 0.07, 0.10))),
                border: Border { color: Color::from_rgba(1.0, 1.0, 1.0, 0.16), width: 1.0, radius: 14.0.into() },
                ..Default::default()
            })
            .into()
    }
}

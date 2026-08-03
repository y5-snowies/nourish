//! `GuideTip`: the guide menu's hover tooltip, rendered as its OWN surface so
//! it floats below the menu instead of being clipped by (or reserving room
//! inside) the menu's texture. Same arrangement as the selection toolbar's tip
//! — the host creates it click-through and hidden, and pushes text into it.
//!
//! One tight line: the entry's shortcut, nothing else.

use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use iced_core::{alignment, Background, Border, Color, Element, Length, Padding, Theme};
use iced_widget::{container, text};

#[derive(Default)]
pub struct GuideTip {
    label: String,
}

#[derive(Debug, Clone)]
pub enum GuideTipMessage {
    Set(String),
}

impl GuideTip {
    pub fn new() -> Self {
        Self::default()
    }
}

impl IcedUi for GuideTip {
    type Message = GuideTipMessage;

    fn update(&mut self, message: Self::Message) {
        let GuideTipMessage::Set(label) = message;
        self.label = label;
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let bubble = container(
            text(self.label.clone())
                .size(13)
                .style(|_t: &Theme| text::Style { color: Some(Color::from_rgb(0.93, 0.95, 0.99)) }),
        )
        .padding(Padding { top: 5.0, bottom: 5.0, left: 11.0, right: 11.0 })
        .style(|_t| container::Style {
            background: Some(Background::Color(Color::from_rgba(0.09, 0.10, 0.13, 0.96))),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.14),
                width: 1.0,
                radius: 7.0.into(),
            },
            ..Default::default()
        });

        // Top-CENTRE anchored: the host centres this surface under the menu, so
        // the bubble hangs from the middle of its top edge whatever its width.
        container(bubble)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Top)
            .into()
    }
}

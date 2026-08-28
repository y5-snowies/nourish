//! The notification pill: one line of text on a dark rounded pill, centred in
//! its surface.
//!
//! Deliberately inert — no buttons, no input. The surface lives in the
//! notification presenter's own registry, which no world hit-tests, so it is
//! click-through by construction; anything interactive here would be a target
//! the user cannot reach. The only state is the message.
//!
//! The ANIMATION is not here. Sliding the pill up from the bottom edge is done
//! by moving the whole surface (`registry.set_location`) once per frame, one
//! integer write, where animating inside the UI would re-render the text every
//! frame for the same result.
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use iced_core::{Background, Border, Color, Element, Length, Theme};
use iced_widget::{container, text};

/// Compositor → surface. One variant: the text to show.
#[derive(Clone, Debug)]
pub enum NotifyMessage {
    SetText(String),
}

#[derive(Default)]
pub struct NotifyUi {
    pub message: String,
}

impl NotifyUi {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl IcedUi for NotifyUi {
    type Message = NotifyMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            NotifyMessage::SetText(t) => self.message = t,
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        container(
            text(self.message.clone())
                .size(14)
                .color(Color::from_rgba(0.94, 0.96, 1.0, 0.96)),
        )
        .padding([10, 18])
        .center_x(Length::Shrink)
        .center_y(Length::Shrink)
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgba(0.06, 0.07, 0.10, 0.88))),
            border: Border {
                radius: 10.0.into(),
                width: 1.0,
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.10),
            },
            ..Default::default()
        })
        .into()
    }
}

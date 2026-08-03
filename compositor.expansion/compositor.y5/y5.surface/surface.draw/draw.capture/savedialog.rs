//! `SaveDialog` — shown when a capture stops: Save / Save As / Discard, with an
//! optional "Optimized encoding" checkbox (video only; hidden when the
//! `background_encoder` setting makes the optimized re-encode automatic).

use iced_core::{Alignment, Background, Border, Element, Length, Shadow, Theme};
use iced_widget::{button, checkbox, column, container, row, text};
use compositor_y5_graphic_capture_session::message::CaptureMessage;
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};

use crate::style;

pub struct SaveDialog {
    /// Label for the artifact ("Screenshot" / "Video").
    pub kind_label: &'static str,
    /// Whether to offer the "Optimized encoding" checkbox (video + manual mode).
    show_optimize: bool,
    /// Current checkbox state (read by the interface on Save).
    optimized: bool,
}

impl SaveDialog {
    pub fn new(kind_label: &'static str, show_optimize: bool) -> Self {
        Self {
            kind_label,
            show_optimize,
            optimized: false,
        }
    }

    /// Whether the user ticked "Optimized encoding".
    pub fn optimized(&self) -> bool {
        self.optimized
    }
}

impl IcedUi for SaveDialog {
    type Message = CaptureMessage;

    fn update(&mut self, message: Self::Message) {
        if let CaptureMessage::ToggleOptimized(v) = message {
            self.optimized = v;
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let title = text(format!("{} captured", self.kind_label))
            .size(20)
            .color(style::TEXT);

        let hint = text("Save to the default folder, choose a location, or discard.")
            .size(13)
            .color(style::TEXT_DIM);

        let buttons = row![
            button(text("Save").size(15).color(style::TEXT))
                .padding([8, 18])
                .on_press(CaptureMessage::SaveDefault)
                .style(style::button_with(style::ACCENT)),
            button(text("Save As…").size(15).color(style::TEXT))
                .padding([8, 18])
                .on_press(CaptureMessage::SaveAs)
                .style(style::button_with(style::BUTTON_BG)),
            button(text("Discard").size(15).color(style::TEXT))
                .padding([8, 18])
                .on_press(CaptureMessage::Discard)
                .style(style::button_with(style::STOP_BG)),
        ]
        .spacing(12);

        let mut col = column![title, hint];
        // The optimized-encode checkbox only applies to video, and only when the
        // background re-encode isn't already automatic.
        if self.show_optimize && self.kind_label == "Video" {
            col = col.push(
                row![
                    checkbox(self.optimized).on_toggle(CaptureMessage::ToggleOptimized),
                    text("Optimized encoding (smaller file, encodes after saving)")
                        .size(13)
                        .color(style::TEXT_DIM),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        col = col.push(buttons).spacing(14).align_x(Alignment::Center);

        let panel = container(col).padding(24).style(|_t| container::Style {
            background: Some(Background::Color(style::PANEL_BG)),
            text_color: Some(style::TEXT),
            border: Border {
                color: style::ACCENT,
                width: 1.0,
                radius: style::RADIUS.into(),
            },
            shadow: Shadow::default(),
            snap: true,
        });

        container(panel)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .into()
    }
}

/// Read back by the capture driver when the dialog is dismissed. One frame of
/// staleness is invisible here: the value only changes on a click, which has
/// already been through a full update+render cycle by the time it is read.
impl compositor_support_iced_core_engine_base::IcedSnapshot for SaveDialog {
    type Snapshot = bool;
    fn snapshot(&self) -> bool {
        self.optimized
    }
}

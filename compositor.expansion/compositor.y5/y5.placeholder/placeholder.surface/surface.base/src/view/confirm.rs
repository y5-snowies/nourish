//! The "container is not running" confirmation prompt.
//!
//! Raised when the user clicks Launch on a placeholder whose plan targets a
//! container the runtime reports as stopped. Starting a container reruns its
//! entrypoint — a side effect well beyond "open this window again" — so it is
//! never done implicitly.
//!
//! Follows the same two arrangements as the placeholder behind it: a ROW at
//! [`Breakpoint::Compact`] (prompt left, answers right) and a centered COLUMN
//! above it. A dialog that didn't respect the breakpoints would be the one
//! view that overflows a placeholder sitting at the layout floor — and it would
//! overflow while blocking the launch the user just asked for.

use iced_core::text::{Ellipsis, Wrapping};
use iced_core::{alignment, Alignment, Element, Length, Theme};
use iced_widget::{button, column, container, row, text};
use compositor_support_iced_core_engine_base::Renderer;

use crate::breakpoint::Breakpoint;
use crate::message::PlaceholderMessage;
use crate::style;
use crate::ui::PlaceholderUi;

pub fn render(
    ui: &PlaceholderUi,
    step: Breakpoint,
) -> Element<'_, PlaceholderMessage, Theme, Renderer> {
    let name = ui.pending_container.clone().unwrap_or_default();

    let content: Element<'_, _, _, _> = if step.is_row() {
        row![
            column![
                line("Container not running", step, style::TEXT, Ellipsis::End),
                line(&name, step, style::TEXT_DIM, Ellipsis::Middle),
            ]
            .spacing(step.line_gap())
            .width(Length::Fill),
            actions(step),
        ]
        .spacing(step.gap())
        .align_y(Alignment::Center)
        .into()
    } else {
        column![
            text("Container not running")
                .size(step.title_size())
                .align_x(alignment::Horizontal::Center)
                .style(|_| iced_widget::text::Style { color: Some(style::TEXT) }),
            line(&name, step, style::TEXT_DIM, Ellipsis::Middle),
            text("Start it before launching? This reruns the container's entrypoint.")
                .size(step.detail_size())
                .width(Length::Fill)
                .align_x(alignment::Horizontal::Center)
                .style(|_| iced_widget::text::Style { color: Some(style::TEXT_HINT) }),
            actions(step),
        ]
        .spacing(step.gap())
        .align_x(Alignment::Center)
        .into()
    };

    container(content)
        .padding(step.outer_padding())
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .into()
}

/// Cancel / confirm. The labels shorten at Compact — two word-labelled buttons
/// plus the prompt do not fit 260px otherwise — but they stay WORDS: this is a
/// consequential yes/no, and a glyph the user has to guess at is not an answer
/// they can give confidently.
fn actions<'a>(step: Breakpoint) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let (no, yes) = if step.is_row() {
        ("No", "Start")
    } else {
        ("Cancel", "Start & Launch")
    };

    row![
        button(text(no).size(step.button_text_size()))
            .padding(step.button_padding())
            .on_press(PlaceholderMessage::CancelContainerStart),
        button(text(yes).size(step.button_text_size()))
            .padding(step.button_padding())
            .on_press(PlaceholderMessage::ContainerStartConfirmed),
    ]
    .spacing(step.button_gap())
    .align_y(Alignment::Center)
    .into()
}

/// A single ellipsized line, matching the placeholder's detail lines.
fn line<'a>(
    value: &str,
    step: Breakpoint,
    color: iced_core::Color,
    ellipsis: Ellipsis,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    text(value.to_string())
        .size(step.detail_size())
        .width(Length::Fill)
        .align_x(if step.is_row() {
            alignment::Horizontal::Left
        } else {
            alignment::Horizontal::Center
        })
        .wrapping(Wrapping::None)
        .ellipsis(ellipsis)
        .style(move |_| iced_widget::text::Style { color: Some(color) })
        .into()
}

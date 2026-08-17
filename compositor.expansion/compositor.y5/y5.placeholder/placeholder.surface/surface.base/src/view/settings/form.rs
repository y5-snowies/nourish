//! The settings form itself. Re-exported from `settings` so
//! `settings::render` is unchanged.
use super::{handler_picker, sections};
use iced_core::{Alignment, Element, Length, Theme};
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
    let actions = row![
        button(text("Cancel").size(step.button_text_size()))
            .padding(step.button_padding())
            .on_press(PlaceholderMessage::CancelSettings),
        button(text("Save").size(step.button_text_size()))
            .padding(step.button_padding())
            .on_press(PlaceholderMessage::SaveClicked {
                updated_plan: Box::new(ui.working.clone()),
            }),
        button(text("Discard").size(step.button_text_size()))
            .padding(step.button_padding())
            .on_press(PlaceholderMessage::RestoreClicked {}),
    ]
    .spacing(step.button_gap())
    .align_y(Alignment::Center);

    // Offered only when a newer sample actually exists, so the button is
    // never a no-op the user has to guess about.
    let actions = if ui.has_pending_sample() {
        actions.push(
            button(text("Pull latest").size(step.button_text_size()))
                .padding(step.button_padding())
                .on_press(PlaceholderMessage::PullSample),
        )
    } else {
        actions
    };

    let title = text("Settings")
        .size(step.title_size())
        .style(|_| iced_widget::text::Style { color: Some(style::TEXT) });

    // A `row` never wraps, so at the narrow steps the title + three buttons
    // run off the right edge instead of shrinking. Stack them instead.
    let header: Element<'_, _, _, _> = if step.shows_title() {
        row![title, actions].spacing(step.button_gap()).align_y(Alignment::Center).into()
    } else {
        column![title, actions].spacing(step.gap()).align_x(Alignment::Start).into()
    };

    let handler_picker = handler_picker::render(ui);

    let identity = sections::render_identity_section(ui);
    let launch = sections::render_launch_section(ui);
    let handler_scoped = sections::render_handler_section(ui);

    let body = column![
        header,
        handler_picker,
        identity,
        launch,
        handler_scoped,
    ]
    .spacing(step.gap())
    .align_x(Alignment::Start);

    container(body)
        .padding(step.outer_padding())
        .width(Length::Fill)
        .into()
}

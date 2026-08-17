//! Render one attribute's row: label + enable toggle + editor + "best inferred" line.

use iced_core::{Alignment, Element, Length, Theme};
use iced_widget::{button, checkbox, column, container, row, text};
use compositor_introspection_extraction_window_base::AttributeDescriptor;
use compositor_support_iced_core_engine_base::Renderer;

use super::attribute_widget;
use crate::message::PlaceholderMessage;
use crate::style;
use crate::ui::PlaceholderUi;

pub fn render<'a>(
    ui: &'a PlaceholderUi,
    descriptor: &AttributeDescriptor,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let label = text(descriptor.label)
        .size(style::TEXT_SIZE_BODY)
        .style(|_| iced_widget::text::Style {
            color: Some(style::TEXT),
        });

    let enabled = ui.working.is_pref_enabled(descriptor);
    let captured = compositor_introspection_launchplan_plan_capture::capture::is_pref_capture(&ui.working, descriptor);
    let desc_key = descriptor.key;

    let enable_toggle: Element<'_, _, _, _> = checkbox(enabled)
        .on_toggle(move |v| PlaceholderMessage::AttributeEnabledChanged {
            descriptor_key: desc_key,
            enabled: v,
        })
        .into();

    // Capture toggle: arm this attribute as a match criterion for adopting a
    // newly-mapped window without an explicit Launch.
    let capture_toggle: Element<'_, _, _, _> = row![
        checkbox(captured).on_toggle(move |v| PlaceholderMessage::AttributeCaptureToggled {
            descriptor_key: desc_key,
            capture: v,
        }),
        text("capture")
            .size(style::TEXT_SIZE_HINT)
            .style(|_| iced_widget::text::Style {
                color: Some(style::TEXT_HINT),
            }),
    ]
    .spacing(4)
    .align_y(Alignment::Center)
    .into();

    // Whether the value below is the user's or the inferred one. The editor
    // renders both identically — it reads `current_raw`, which is
    // override-or-inferred — so without this there is no way to tell that a
    // field holds an override at all, let alone to remove one.
    let overridden = ui.working.has_pref_override(descriptor);
    let override_marker: Element<'_, _, _, _> = if overridden {
        row![
            text("overridden")
                .size(style::TEXT_SIZE_HINT)
                .style(|_| iced_widget::text::Style { color: Some(style::ACCENT) }),
            // The one gesture that means "use the inferred value". Clearing
            // the editor text cannot mean it: that has to keep meaning
            // "override with an empty value".
            button(text("revert").size(style::TEXT_SIZE_HINT))
                .padding(style::PAD_SMALL)
                .on_press(PlaceholderMessage::AttributeOverrideCleared {
                    descriptor_key: desc_key,
                }),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
    } else {
        text("inferred")
            .size(style::TEXT_SIZE_HINT)
            .style(|_| iced_widget::text::Style { color: Some(style::TEXT_HINT) })
            .into()
    };

    let header = row![enable_toggle, label, capture_toggle, override_marker]
        .spacing(8)
        .align_y(Alignment::Center);

    let editor: Element<'_, _, _, _> = if enabled {
        attribute_widget::render(ui, descriptor)
    } else {
        // Disabled: editor grayed out / not interactive. Show
        // a brief explanation.
        text("(disabled — won't be passed to the launched process)")
            .size(style::TEXT_SIZE_HINT)
            .style(|_| iced_widget::text::Style {
                color: Some(style::TEXT_HINT),
            })
            .into()
    };

    let best_line = render_best_hint_line(ui, descriptor);

    // Informative only — no revert. The refreshed value IS the correct one;
    // this just explains why the field moved without the user touching it.
    let mut inner = column![header, editor, best_line]
        .spacing(6)
        .align_x(Alignment::Start);

    if let Some(note) = ui.refresh_note(descriptor.key) {
        inner = inner.push(
            text(note)
                .size(style::TEXT_SIZE_HINT)
                .style(|_| iced_widget::text::Style { color: Some(style::ACCENT) }),
        );
    }

    container(inner)
        .padding(style::PAD_SMALL)
        .width(Length::Fill)
        .into()
}

fn render_best_hint_line<'a>(
    ui: &'a PlaceholderUi,
    descriptor: &AttributeDescriptor,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let best = ui.working.best_raw(descriptor);
    let summary = match best {
        Some(arc) => format!("Best inferred: {}", attribute_widget::summarize_value(&arc)),
        None => "Best inferred: (none)".to_string(),
    };
    text(summary)
        .size(style::TEXT_SIZE_HINT)
        .style(|_| iced_widget::text::Style {
            color: Some(style::TEXT_HINT),
        })
        .into()
}

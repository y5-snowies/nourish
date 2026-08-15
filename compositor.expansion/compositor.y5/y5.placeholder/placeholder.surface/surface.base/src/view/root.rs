//! Top-level view. Re-exported from `view` so `view::root_view` is unchanged.
use super::{settings, view_mode};
use iced_core::{Element, Length, Theme};
use iced_widget::{container, scrollable};
use compositor_support_iced_core_engine_base::Renderer;

use crate::message::PlaceholderMessage;
use crate::mode::Mode;
use crate::style;
use crate::ui::PlaceholderUi;

/// Top-level view. Always wrapped in a Scrollable so the whole UI is
/// usable even when the surface is cropped by the compositor.
///
/// The outer container has heavily rounded corners and a soft drop
/// shadow to give the placeholder a "floating panel" feel.
pub fn root_view(ui: &PlaceholderUi) -> Element<'_, PlaceholderMessage, Theme, Renderer> {
    let body: Element<'_, _, _, _> = match ui.mode {
        Mode::View => view_mode::render(ui),
        Mode::Settings => settings::render(ui),
    };

    let scroll = scrollable(body)
        .width(Length::Fill)
        .height(Length::Fill);

    container(scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced_core::Background::Color(style::BG)),
            text_color: Some(style::TEXT),
            border: iced_core::Border {
                color: style::BORDER,
                width: 1.0,
                radius: style::RADIUS_LARGE.into(),
            },
            shadow: iced_core::Shadow {
                color: iced_core::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.6 },
                offset: iced_core::Vector::new(0.0, 12.0),
                blur_radius: 32.0,
            },
            snap: true,
        })
        .into()
}

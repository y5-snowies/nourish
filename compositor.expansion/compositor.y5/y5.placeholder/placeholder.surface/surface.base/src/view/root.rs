//! Top-level view. Re-exported from `view` so `view::root_view` is unchanged.
use super::{confirm, settings, view_mode};
use iced_core::text::{Ellipsis, Wrapping};
use iced_core::{alignment, Element, Length, Theme};
use iced_widget::{container, responsive, scrollable, stack, text};
use compositor_introspection_launchplan_plan_container::container as plan_container;
use compositor_support_iced_core_engine_base::Renderer;

use crate::breakpoint::{self, Breakpoint};
use crate::message::PlaceholderMessage;
use crate::mode::Mode;
use crate::style;
use crate::ui::PlaceholderUi;

/// Marks a containerised placeholder. Nested squares read as containment and
/// are present in the font stack already in use, unlike an emoji box.
const CONTAINER_GLYPH: &str = "▣";
/// Session badge. Same geometric-shapes block as the container glyph, so the two
/// share a weight and a font that certainly has both.
const SESSION_GLYPH: &str = "◈";

/// Top-level view.
///
/// The body is built inside a [`responsive`] so the layout can pick a
/// [`Breakpoint`] from the space the compositor actually gave the tile — a
/// placeholder is dragged to arbitrary sizes and the roomy layout does not
/// fit a short tile at all. The responsive sits OUTSIDE any Scrollable on
/// purpose: a scrollable hands its content an unbounded axis, so measuring
/// inside one would report infinite height and always resolve to
/// [`Breakpoint::Full`].
///
/// Scrolling is applied only where it is actually needed, because an
/// unbounded axis also defeats `Length::Fill` — content inside a vertical
/// scrollable can never center vertically, it can only stack from the top:
///
/// - **Settings** is an arbitrarily long form: always vertically scrollable.
/// - **View** is designed to fit its step exactly, so it gets NO scrollable
///   and centers properly. The one exception is a tile below
///   [`breakpoint::MIN_W`]/[`breakpoint::MIN_H`] — the placeholder state
///   clamps geometry to that floor, so this only happens to a record written
///   before the clamp existed, or to a surface the compositor crops. There a
///   two-axis scrollable goes back on so nothing becomes unreachable.
///
/// The outer container has heavily rounded corners and a soft drop
/// shadow to give the placeholder a "floating panel" feel.
pub fn root_view(ui: &PlaceholderUi) -> Element<'_, PlaceholderMessage, Theme, Renderer> {
    // Read from the canonical plan, not `shown_plan`: which container an app
    // lives in is a fact about the captured window, and it should not flicker
    // while the user edits an unrelated attribute in Settings.
    //
    // Both answers come from the shared plan query, which is also what the
    // launch path uses — so the badge can never claim a container the launch
    // would not actually target.
    let container_name = plan_container::container_label(&ui.canonical);
    let in_container = plan_container::is_containerised(&ui.canonical);
    // Not read from the plan: a session identity is declared by the CLIENT over
    // `xdg-session-management-v1` and lives on the placeholder record, not in
    // anything the launch plan describes.
    let in_session = ui.has_session_identity;

    // iced master infers `responsive`'s closure return less eagerly; name the type.
    let body = responsive(move |size| -> Element<'_, PlaceholderMessage, Theme, Renderer> {
        let step = Breakpoint::of(size);

        let content: Element<'_, _, _, _> = match ui.mode {
            Mode::Settings => scrollable(settings::render(ui, step))
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            Mode::View | Mode::ConfirmContainerStart => {
                let body = match ui.mode {
                    Mode::ConfirmContainerStart => confirm::render(ui, step),
                    _ => view_mode::render(ui, step),
                };
                if size.width >= breakpoint::MIN_W && size.height >= breakpoint::MIN_H {
                    body
                } else {
                    scrollable(body)
                        .direction(scrollable::Direction::Both {
                            vertical: scrollable::Scrollbar::default(),
                            horizontal: scrollable::Scrollbar::default(),
                        })
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into()
                }
            }
        };

        if !in_container && !in_session {
            return content;
        }

        // Overlays, not layout. `stack`'s base layer alone sizes the stack
        // and every later layer is laid out inside `Limits::new(ZERO, size)`,
        // so these cost the content no space and shift no padding. Being
        // siblings of the scrollable rather than children of it is what keeps
        // them still while the body scrolls.
        //
        // Built as a Vec because the two markers are independent: a tile can be
        // containerised, session-bearing, or both, and `stack!` needs a fixed
        // arity. The badges sit in opposite top corners so both are legible when
        // both apply.
        let mut layers: Vec<Element<'_, _, _, _>> = vec![content];
        if in_container {
            layers.push(container_badge(step));
            layers.push(container_name_floater(container_name.clone(), step));
        }
        if in_session {
            layers.push(session_badge(step));
        }
        stack(layers).into()
    });

    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            // Four states, each its own tone — see `style::BG_CONTAINER_SESSION`
            // for why "both" is not a blend of the two singles.
            background: Some(iced_core::Background::Color(match (in_container, in_session) {
                (true, true) => style::BG_CONTAINER_SESSION,
                (true, false) => style::BG_CONTAINER,
                (false, true) => style::BG_SESSION,
                (false, false) => style::BG,
            })),
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

/// Corner badge marking the placeholder as containerised. Drawn at every
/// breakpoint — a tile at the layout floor is exactly where you most need to
/// know the app is not on the host.
fn container_badge<'a>(step: Breakpoint) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    container(
        text(CONTAINER_GLYPH)
            .size(step.badge_px())
            .style(|_| iced_widget::text::Style { color: Some(style::CONTAINER_ACCENT) }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(step.overlay_inset())
    .align_x(alignment::Horizontal::Right)
    .align_y(alignment::Vertical::Top)
    .into()
}

/// The session marker, top LEFT — opposite the container badge, so a tile that
/// is both shows both without either overlapping the other.
fn session_badge<'a>(step: Breakpoint) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    container(
        text(SESSION_GLYPH)
            .size(step.badge_px())
            .style(|_| iced_widget::text::Style { color: Some(style::SESSION_ACCENT) }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(step.overlay_inset())
    .align_x(alignment::Horizontal::Left)
    .align_y(alignment::Vertical::Top)
    .into()
}

/// The container name, floating at bottom centre.
///
/// Shows the name when we have one and a short id when we do not — see
/// `plan_container::container_label`. The remaining `None` arm is a plan that
/// says it is containerised while carrying neither identifier, which the launch
/// could not act on either. Saying so beats showing nothing, which would read as
/// "not in a container" and is the opposite of the truth.
fn container_name_floater<'a>(
    name: Option<String>,
    step: Breakpoint,
) -> Element<'a, PlaceholderMessage, Theme, Renderer> {
    let (label, colour) = match name {
        Some(name) => (name, style::CONTAINER_ACCENT),
        None => ("container unidentified".to_string(), style::CONTAINER_DIM),
    };

    container(
        text(label)
            .size(step.detail_size())
            .wrapping(Wrapping::None)
            .ellipsis(Ellipsis::Middle)
            .style(move |_| iced_widget::text::Style { color: Some(colour) }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(step.overlay_inset())
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Bottom)
    .into()
}

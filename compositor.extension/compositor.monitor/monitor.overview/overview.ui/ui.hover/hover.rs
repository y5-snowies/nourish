//! `WindowCard`: the overview grid's hover label — the app icon and the window
//! title, floating over the thumbnail the cursor is on.
//!
//! Its own surface (a click-through tooltip), like `GuideTip`: the grid draws
//! live window thumbnails, so there is nowhere inside that content to put a
//! label. The host owns everything about WHEN it shows and WHERE; this is only
//! the bubble, and it is pushed one `Set` per hovered window.
//!
//! The icon arrives already decided — a file the host resolved, or the raw
//! pixels a client handed over through `xdg_toplevel_icon_v1`. This crate does
//! no lookup of its own and simply shows nothing when the host had nothing.

use std::path::PathBuf;
use std::sync::Arc;
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};
use iced_core::{alignment, Background, Border, Bytes, Color, ContentFit, Element, Length, Padding, Theme};
use iced_widget::{container, image, svg, text, Row, Space};

/// Icon edge, logical px. Matches the card's text line so the row reads flat.
const ICON: f32 = 22.0;

/// Where the card's icon comes from. Both forms are already resolved: the host
/// does the icon-theme lookup and the buffer decode.
#[derive(Debug, Clone)]
pub enum CardIcon {
    /// An icon file on disk (a desktop entry's, or a resolved icon name).
    File(PathBuf),
    /// Straight `RGBA8888` pixels, `width * height * 4` bytes — what a client
    /// attached to its toplevel over `xdg_toplevel_icon_v1`.
    Pixels { width: u32, height: u32, rgba: Arc<Vec<u8>> },
}

/// Same icon = same file, or the same decoded buffer. Comparing the pixels
/// themselves is what the `Arc` exists to avoid — this runs per frame.
impl PartialEq for CardIcon {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CardIcon::File(a), CardIcon::File(b)) => a == b,
            (
                CardIcon::Pixels { width: aw, height: ah, rgba: a },
                CardIcon::Pixels { width: bw, height: bh, rgba: b },
            ) => aw == bw && ah == bh && Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// The icon as iced holds it. Built once per `Set` rather than per `view`:
/// `image::Handle::from_rgba` mints a fresh id each call, which would re-upload
/// the texture on every re-render.
enum Art {
    Image(image::Handle),
    Svg(svg::Handle),
    None,
}

pub struct WindowCard {
    title: String,
    art: Art,
}

impl Default for WindowCard {
    fn default() -> Self {
        Self { title: String::new(), art: Art::None }
    }
}

#[derive(Debug, Clone)]
pub enum WindowCardMessage {
    /// Replace the whole content: title, and the icon (if any).
    Set { title: String, icon: Option<CardIcon> },
}

impl WindowCard {
    pub fn new() -> Self {
        Self::default()
    }
}

impl IcedUi for WindowCard {
    type Message = WindowCardMessage;

    fn update(&mut self, message: Self::Message) {
        let WindowCardMessage::Set { title, icon } = message;
        self.title = title;
        self.art = match icon {
            Some(CardIcon::Pixels { width, height, rgba }) => Art::Image(image::Handle::from_rgba(
                width,
                height,
                Bytes::from(rgba.as_ref().clone()),
            )),
            Some(CardIcon::File(path)) if is_svg(&path) => Art::Svg(svg::Handle::from_path(path)),
            Some(CardIcon::File(path)) => Art::Image(image::Handle::from_path(path)),
            None => Art::None,
        };
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        // No content = draw NOTHING, rather than the host hiding the surface.
        // A hidden instance has its GPU ring released by the worker, and a
        // released ring turns the next content change into a repaint that needs
        // two more compositor frames to appear — frames nobody asks for once the
        // pointer stops. An empty transparent view looks identical and keeps the
        // surface live, so every later change renders on the spot.
        if self.title.is_empty() && matches!(self.art, Art::None) {
            return container(Space::new()).width(Length::Fill).height(Length::Fill).into();
        }
        let mut row: Vec<Element<'_, Self::Message, Theme, Renderer>> = Vec::new();
        match &self.art {
            Art::Image(handle) => row.push(
                image(handle.clone())
                    .width(Length::Fixed(ICON))
                    .height(Length::Fixed(ICON))
                    .content_fit(ContentFit::Contain)
                    .into(),
            ),
            Art::Svg(handle) => row.push(
                svg(handle.clone())
                    .width(Length::Fixed(ICON))
                    .height(Length::Fixed(ICON))
                    .content_fit(ContentFit::Contain)
                    .into(),
            ),
            Art::None => {}
        }
        row.push(
            text(self.title.clone())
                .size(14)
                // One line, clipped with a tail — the card is a fixed width and a
                // window title can be a whole file path.
                .wrapping(text::Wrapping::None)
                .ellipsis(text::Ellipsis::End)
                .style(|_t: &Theme| text::Style { color: Some(Color::from_rgb(0.93, 0.95, 0.99)) })
                .into(),
        );

        let bubble = container(Row::with_children(row).spacing(9).align_y(alignment::Vertical::Center))
            .padding(Padding { top: 6.0, bottom: 6.0, left: 10.0, right: 12.0 })
            .style(|_t| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.09, 0.10, 0.13, 0.94))),
                border: Border {
                    color: Color::from_rgba(1.0, 1.0, 1.0, 0.14),
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..Default::default()
            });

        // Bottom-CENTRE anchored: the host centres this surface on the hovered
        // cell, so the bubble sits on the thumbnail's lower edge whatever its width.
        container(bubble)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Bottom)
            .into()
    }
}

/// SVG and raster icons need different iced widgets; the extension decides.
fn is_svg(path: &PathBuf) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("svg"))
        .unwrap_or(false)
}

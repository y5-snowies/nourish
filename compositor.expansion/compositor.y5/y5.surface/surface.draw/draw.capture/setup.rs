//! `SetupOverlay` — the "ice screen" setup phase. A full-screen screen-space
//! instance: opaque black mask with a clear hole over the prospective capture
//! target, plus a chooser (target kind + media + Confirm/Cancel). For the two
//! region kinds the hole is drawn by dragging; for Windows it mirrors the
//! pre-selected canvas windows' bbox; for FullScreen it's the whole output.
//!
//! The overlay is authoritative for the chosen kind/media/draft — the interface
//! reads them off the instance on Confirm (it reads the live canvas selection
//! itself for the Windows case).

use iced_core::{Alignment, Background, Border, Element, Length, Shadow, Theme};
use iced_widget::{button, column, container, mouse_area, row, text};
use compositor_y5_graphic_capture_session::message::{
    CaptureMedia, CaptureMessage, OverlayPoint, OverlayRect, TargetKind,
};
use compositor_support_iced_core_engine_base::ui::EventFlags;
use compositor_support_iced_core_engine_base::{IcedEvent, IcedUi, Renderer};

use crate::{mask, style};

pub struct SetupOverlay {
    screen_w: i32,
    screen_h: i32,
    kind: TargetKind,
    media: CaptureMedia,
    /// In-progress region rect for the region kinds (screen pixels).
    draft: Option<OverlayRect>,
    /// Bbox of the pre-selected canvas windows (screen pixels), if any.
    preselect: Option<OverlayRect>,
    /// Transparent-background option (window/world targets). Default off.
    no_background: bool,
    dragging: bool,
    drag_origin: (i32, i32),
    cursor: (i32, i32),
}

impl SetupOverlay {
    pub fn new(
        screen_w: i32,
        screen_h: i32,
        media: CaptureMedia,
        kind: TargetKind,
        preselect: Option<OverlayRect>,
    ) -> Self {
        Self {
            screen_w,
            screen_h,
            kind,
            media,
            draft: None,
            preselect,
            no_background: false,
            dragging: false,
            drag_origin: (0, 0),
            cursor: (0, 0),
        }
    }

    pub fn kind(&self) -> TargetKind {
        self.kind
    }
    pub fn media(&self) -> CaptureMedia {
        self.media
    }
    pub fn draft(&self) -> Option<OverlayRect> {
        self.draft
    }
    pub fn no_background(&self) -> bool {
        self.no_background
    }

    /// The no-background toggle is only meaningful for window/world targets
    /// (screen/full-screen capture the composited screen, backdrop included).
    fn supports_no_background(&self) -> bool {
        matches!(self.kind, TargetKind::Windows | TargetKind::WorldRegion)
    }

    fn is_region(&self) -> bool {
        matches!(self.kind, TargetKind::WorldRegion | TargetKind::ScreenRegion)
    }

    /// The hole to clear in the mask for the current selection.
    fn hole(&self) -> Option<OverlayRect> {
        match self.kind {
            TargetKind::Windows => self.preselect,
            TargetKind::WorldRegion | TargetKind::ScreenRegion => self.draft,
            TargetKind::FullScreen => Some(OverlayRect {
                x: 0,
                y: 0,
                w: self.screen_w,
                h: self.screen_h,
            }),
        }
    }
}

fn rect_from(a: (i32, i32), b: (i32, i32)) -> OverlayRect {
    let x = a.0.min(b.0);
    let y = a.1.min(b.1);
    OverlayRect {
        x,
        y,
        w: (a.0 - b.0).abs(),
        h: (a.1 - b.1).abs(),
    }
}

impl IcedUi for SetupOverlay {
    type Message = CaptureMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            CaptureMessage::DragMove(OverlayPoint { x, y }) => {
                self.cursor = (x as i32, y as i32);
                if self.dragging && self.is_region() {
                    self.draft = Some(rect_from(self.drag_origin, self.cursor));
                }
            }
            CaptureMessage::DragStart => {
                if self.is_region() {
                    self.dragging = true;
                    self.drag_origin = self.cursor;
                    self.draft = Some(rect_from(self.cursor, self.cursor));
                }
            }
            CaptureMessage::DragEnd => {
                self.dragging = false;
            }
            CaptureMessage::SelectKind(k) => {
                self.kind = k;
            }
            CaptureMessage::SelectMedia(m) => {
                self.media = m;
            }
            CaptureMessage::SetNoBackground(v) => {
                self.no_background = v;
            }
            // Confirm/Cancel and everything else are handled by the interface.
            _ => {}
        }
    }

    fn subscribe(&self) -> EventFlags {
        EventFlags::KEYBOARD
    }

    fn event_process(&self, event: &IcedEvent) -> Vec<Self::Message> {
        use iced_core::keyboard::{self, Key, key::Named};
        if let IcedEvent::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event {
            match key {
                Key::Named(Named::Escape) => return vec![CaptureMessage::Cancel],
                Key::Named(Named::Enter) => return vec![CaptureMessage::Confirm],
                _ => {}
            }
        }
        Vec::new()
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        // Bottom layer: the mask with a clear hole, wrapped so the whole area
        // drives the region drag. The chooser (top layer) intercepts its own
        // clicks first, so dragging only starts on empty/masked area.
        let mask_layer = mouse_area(mask::mask_with_hole(
            self.screen_w,
            self.screen_h,
            self.hole(),
            style::SETUP_MASK,
            Some((style::ACCENT, style::BORDER_WIDTH)),
        ))
        .on_press(CaptureMessage::DragStart)
        .on_release(CaptureMessage::DragEnd)
        .on_move(|p| CaptureMessage::DragMove(OverlayPoint { x: p.x, y: p.y }));

        let controls = container(self.chooser())
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .padding(28);

        iced_widget::stack![mask_layer, controls].into()
    }
}

impl SetupOverlay {
    fn chooser(&self) -> Element<'_, CaptureMessage, Theme, Renderer> {
        let kinds = row![
            kind_button("Windows", TargetKind::Windows, self.kind),
            kind_button("World region", TargetKind::WorldRegion, self.kind),
            kind_button("Screen region", TargetKind::ScreenRegion, self.kind),
            kind_button("Full screen", TargetKind::FullScreen, self.kind),
        ]
        .spacing(8);

        let media = row![
            media_button("Screenshot", CaptureMedia::Screenshot, self.media),
            media_button("Video", CaptureMedia::Video, self.media),
        ]
        .spacing(8);

        // Transparent-background toggle (window/world only).
        let options: Element<'_, CaptureMessage, Theme, Renderer> = if self.supports_no_background() {
            row![toggle_button(
                "Transparent background",
                self.no_background,
                CaptureMessage::SetNoBackground(!self.no_background),
            )]
            .spacing(8)
            .into()
        } else {
            iced_widget::Space::new().into()
        };

        let actions = row![
            button(text("Confirm").size(15).color(style::TEXT))
                .padding([8, 18])
                .on_press(CaptureMessage::Confirm)
                .style(style::button_with(style::ACCENT)),
            button(text("Cancel").size(15).color(style::TEXT))
                .padding([8, 18])
                .on_press(CaptureMessage::Cancel)
                .style(style::button_with(style::BUTTON_BG)),
        ]
        .spacing(8);

        let hint = text(match self.kind {
            TargetKind::Windows => "Capturing the selected windows.",
            TargetKind::WorldRegion => "Drag to draw a region (moves with the world).",
            TargetKind::ScreenRegion => "Drag to draw a region (fixed on screen).",
            TargetKind::FullScreen => "Capturing the whole screen.",
        })
        .size(13)
        .color(style::TEXT_DIM);

        let panel = column![
            text("Capture setup").size(18).color(style::TEXT),
            kinds,
            media,
            options,
            hint,
            actions,
        ]
        .spacing(12)
        .align_x(Alignment::Center);

        container(panel)
            .padding(20)
            .style(|_t| container::Style {
                background: Some(Background::Color(style::PANEL_BG)),
                text_color: Some(style::TEXT),
                border: Border {
                    color: style::ACCENT,
                    width: 1.0,
                    radius: style::RADIUS.into(),
                },
                shadow: Shadow::default(),
                snap: true,
            })
            .into()
    }
}

fn kind_button<'a>(
    label: &'a str,
    kind: TargetKind,
    selected: TargetKind,
) -> Element<'a, CaptureMessage, Theme, Renderer> {
    let bg = if kind == selected {
        style::ACCENT
    } else {
        style::BUTTON_BG
    };
    button(text(label).size(14).color(style::TEXT))
        .padding([7, 12])
        .on_press(CaptureMessage::SelectKind(kind))
        .style(style::button_with(bg))
        .into()
}

fn media_button<'a>(
    label: &'a str,
    media: CaptureMedia,
    selected: CaptureMedia,
) -> Element<'a, CaptureMessage, Theme, Renderer> {
    let bg = if media == selected {
        style::ACCENT
    } else {
        style::BUTTON_BG
    };
    button(text(label).size(14).color(style::TEXT))
        .padding([7, 12])
        .on_press(CaptureMessage::SelectMedia(media))
        .style(style::button_with(bg))
        .into()
}

/// A checkbox-style toggle button (accent when on).
fn toggle_button<'a>(
    label: &'a str,
    on: bool,
    msg: CaptureMessage,
) -> Element<'a, CaptureMessage, Theme, Renderer> {
    let bg = if on { style::ACCENT } else { style::BUTTON_BG };
    let mark = if on { "☑" } else { "☐" };
    button(
        text(format!("{mark}  {label}"))
            .size(14)
            .color(style::TEXT),
    )
    .padding([7, 12])
    .on_press(msg)
    .style(style::button_with(bg))
    .into()
}

/// What the capture driver reads off this overlay when the user commits.
#[derive(Clone, Debug)]
pub struct SetupSnapshot {
    pub kind: TargetKind,
    pub media: CaptureMedia,
    pub draft: Option<OverlayRect>,
    pub no_background: bool,
}

/// These fields only move on a click, which has already been through a full
/// update+render cycle by the time the driver reads them, so the one frame of
/// staleness an off-thread snapshot carries is not observable.
impl compositor_support_iced_core_engine_base::IcedSnapshot for SetupOverlay {
    type Snapshot = SetupSnapshot;
    fn snapshot(&self) -> SetupSnapshot {
        SetupSnapshot {
            kind: self.kind,
            media: self.media,
            draft: self.draft,
            no_background: self.no_background,
        }
    }
}

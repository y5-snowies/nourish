use crate::app::CompositorSnapshot;
use iced_core::{
    Background, Border, Color, Element, Length, Padding, Shadow, Theme, Vector, alignment,
};
use iced_wgpu::Renderer;
use iced_widget::{column, container};
use crate::selection::{CloseMode, ScaleToFitOption, SelectionAction, SelectionState, TipKind};
use compositor_support_iced_core_engine_base::IcedUi;
use compositor_support_iced_core_engine_base::ui::EventFlags;

pub struct Overlay {
    pub clicks: u32,
    pub last_pressed: Option<u8>,
    pub mode: Mode,
    pub selection_count: u32,
    pub selection: SelectionState,
    /// Which button the pointer is currently over (drives the separate tooltip
    /// surface). `None` when nothing is hovered.
    pub hovered: Option<TipKind>,
}

pub enum Mode {
    Selecting,
    None,
}

#[derive(Debug, Clone)]
pub enum Message {
    ButtonPressed(u8),
    SelectionAction(SelectionAction),
    SelectNotify(i32),
    SelectionClicked(SelectionAction),
    ClickedOutside,
    ShiftChanged(bool),
    AltChanged(bool),
    ExecuteSelection(Vec<SelectionAction>, bool),
    ExecuteScaleToFit(ScaleToFitOption),
    /// Pointer entered (`Some`) or left (`None`) a button — drives the tooltip.
    Hover(Option<TipKind>),
    /// Close every selected window at the strength chosen by the modifiers held
    /// at click time (none = protocol close, Shift = close + no placeholder,
    /// Alt = SIGTERM, Alt+Shift = SIGKILL).
    CloseSelected(CloseMode),
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            selection_count: 0,
            clicks: 0,
            last_pressed: None,
            mode: Mode::None,
            selection: SelectionState::default(),
            hovered: None,
        }
    }
}

impl Overlay {
    pub fn with_snapshot(_snapshot: CompositorSnapshot) -> Self {
        Self::default()
    }

    /// Build with the current selection count so the bar renders immediately
    /// (the reconciler creates the instance only when count > 0).
    pub fn with_count(count: i32) -> Self {
        let mut o = Self::default();
        o.apply_count(count);
        o
    }

    fn apply_count(&mut self, size: i32) {
        self.mode = if size == 0 { Mode::None } else { Mode::Selecting };
        self.selection_count = size as u32;
    }
}

impl IcedUi for Overlay {
    type Message = Message;

    fn subscribe(&self) -> EventFlags {
        // Shift/Alt arrive as keyboard `ModifiersChanged` events; button clicks
        // are consumed by the iced widgets directly.
        EventFlags::KEYBOARD
    }

    fn event_process(&self, event: &iced_core::Event) -> Vec<Message> {
        if let iced_core::Event::Keyboard(iced_core::keyboard::Event::ModifiersChanged(mods)) =
            event
        {
            return vec![
                Message::ShiftChanged(mods.shift()),
                Message::AltChanged(mods.alt()),
            ];
        }
        Vec::new()
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::ButtonPressed(n) => {
                self.clicks += 1;
                self.last_pressed = Some(n);
                info!("button pressed button={n} total_clicks={}", self.clicks);
            }
            Message::SelectNotify(size) => self.apply_count(size),
            Message::ShiftChanged(held) => self.selection.shift_held = held,
            Message::AltChanged(held) => self.selection.alt_held = held,
            Message::Hover(kind) => self.hovered = kind,
            Message::SelectionClicked(action) => {
                if self.selection.shift_held {
                    self.handle_shift_click(action);
                } else {
                    self.handle_plain_click(action);
                }
            }
            Message::ClickedOutside => self.selection.clear(),
            Message::ExecuteSelection(_, _) => {
                self.selection.clear();
            }
            _ => {}
        }
    }

    fn view(&self) -> Element<'_, Message, Theme, Renderer> {
        let bar_element: Element<'_, Message, Theme, Renderer> = match self.mode {
            Mode::None => container(column!()).into(),
            Mode::Selecting => {
                let bar = self.selection();
                container(bar)
                    .padding(Padding { top: 10.0, right: 16.0, bottom: 10.0, left: 16.0 })
                    .style(|_theme| container::Style {
                        snap: true,
                        background: Some(Background::Color(Color::WHITE)),
                        border: Border {
                            color: Color::from_rgba(0.0, 0.0, 0.0, 0.08),
                            width: 1.0,
                            radius: 14.0.into(),
                        },
                        shadow: Shadow {
                            color: Color::from_rgba(0.0, 0.0, 0.0, 0.18),
                            offset: Vector::new(0.0, 4.0),
                            blur_radius: 18.0,
                        },
                        text_color: Some(Color::from_rgb(0.1, 0.12, 0.16)),
                    })
                    .into()
            }
        };

        // Bottom-anchored bar, centered horizontally. (No synthetic cursor: the
        // compositor draws the real cursor over this surface.)
        container(bar_element)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Bottom)
            .padding(Padding { top: 0.0, right: 0.0, bottom: 8.0, left: 0.0 })
            .into()
    }
}

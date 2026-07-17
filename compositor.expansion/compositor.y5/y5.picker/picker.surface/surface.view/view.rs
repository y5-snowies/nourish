//! Bottom-right details panel: a big borderless world name (click to edit) and a
//! small Delete with inline confirmation. No background — it floats over the
//! scene, right-aligned.
use iced_core::alignment::Horizontal;
use iced_core::{Background, Border, Color, Element, Length, Theme};
use iced_widget::{button, column, container, row, text, text_input, Space};
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};

/// Messages between the panel and the compositor. `SetWorld` is compositor →
/// surface; `Enter` starts the selected world; the `Delete*` trio is the
/// request → confirm/cancel flow.
#[derive(Clone, Debug)]
pub enum PickerSurfaceMessage {
    SetWorld { name: String, can_delete: bool },
    NameEdited(String),
    Enter,
    DeleteRequest,
    DeleteConfirm,
    DeleteCancel,
    /// The touch-reachable Close: cancel the picker, back to the origin world.
    Close,
}

#[derive(Default)]
pub struct PickerSurface {
    pub name: String,
    pub can_delete: bool,
    pub confirming: bool,
}

impl PickerSurface {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A small borderless text button (the Delete / Yes / No actions).
fn tap(s: &str, msg: PickerSurfaceMessage) -> Element<'_, PickerSurfaceMessage, Theme, Renderer> {
    button(text(s).size(13))
        .padding(0)
        .on_press(msg)
        .style(|_t: &Theme, _s| button::Style {
            text_color: Color::from_rgba(0.85, 0.9, 1.0, 0.9), ..Default::default()
        })
        .into()
}

impl IcedUi for PickerSurface {
    type Message = PickerSurfaceMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            PickerSurfaceMessage::SetWorld { name, can_delete } => {
                (self.name, self.can_delete, self.confirming) = (name, can_delete, false);
            }
            PickerSurfaceMessage::NameEdited(n) => self.name = n,
            PickerSurfaceMessage::DeleteRequest => self.confirming = true,
            PickerSurfaceMessage::DeleteCancel => self.confirming = false,
            PickerSurfaceMessage::Enter
            | PickerSurfaceMessage::DeleteConfirm
            | PickerSurfaceMessage::Close => {}
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        let name = text_input("Unnamed", &self.name)
            .on_input(PickerSurfaceMessage::NameEdited)
            .on_submit(PickerSurfaceMessage::Enter)
            .size(28).padding(0).align_x(Horizontal::Right).width(Length::Fill)
            .style(|_t: &Theme, _s| text_input::Style {
                background: Background::Color(Color::TRANSPARENT), border: Border::default(),
                icon: Color::TRANSPARENT, placeholder: Color::from_rgba(1.0, 1.0, 1.0, 0.35),
                value: Color::WHITE, selection: Color::from_rgba(0.4, 0.6, 1.0, 0.4),
            });

        let action: Element<'_, _, _, _> = if !self.can_delete {
            Space::new().into()
        } else if self.confirming {
            row![
                text("Delete?").size(13).color(Color::from_rgba(1.0, 0.7, 0.7, 0.9)),
                Space::new().width(10),
                tap("Yes", PickerSurfaceMessage::DeleteConfirm),
                Space::new().width(10),
                tap("No", PickerSurfaceMessage::DeleteCancel),
            ].into()
        } else {
            tap("Delete", PickerSurfaceMessage::DeleteRequest)
        };

        container(
            column![
                name,
                Space::new().height(6),
                action,
                Space::new().height(6),
                tap("Close", PickerSurfaceMessage::Close),
            ]
            .align_x(Horizontal::Right),
        )
        .width(Length::Fill).height(Length::Fill).align_x(Horizontal::Right).into()
    }
}

//! The trait every Iced UI in this system implements.
//!
//! This is a deliberately minimal contract. No async, no `Task`, no
//! `Subscription`, no `Application`. The compositor drives lifecycle
//! externally; the UI just describes its widget tree and reacts to messages.

use iced_core::Element;
use iced_core::Theme;
use iced_wgpu::Renderer;

/// One Iced UI definition.
///
/// Implementors are `Send` so that the registry can hold them behind a
/// trait object across threads if needed. They're `'static` because Iced's
/// internal types (`Cache`, `Tree`) are themselves `'static`-bound.
///
/// ## Why no `Task`?
/// Stripped per spec. If your UI needs to do work, the compositor performs
/// it: the UI emits a message, the compositor's `handle_message` callback
/// (set up via `IcedRuntime::set_message_handler`) sees it, does whatever
/// async/IO work is needed, then dispatches new messages back via
/// `queue_message`. Bidirectional, but explicit.
///
/// ## Why associated `Message` and not generic?
/// One UI type → one message type. The registry uses the associated type
/// to enforce routing correctness at the type level: `IcedHandle<U>` can
/// only dispatch `U::Message`. No string-keyed protocol, no dynamic
/// dispatch on messages.

pub trait IcedUi: Send + 'static {
    type Message: Clone + Send + std::fmt::Debug + 'static;

    /// Build the widget tree. Called by the runtime on every tick and
    /// every render — Iced's `Cache` makes repeated calls cheap.
    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer>;

    /// Apply a message to the UI's state. No return value (`Task::none()`
    /// is the implicit answer everywhere).
    fn update(&mut self, message: Self::Message);

    fn subscribe(&self) -> EventFlags {
        EventFlags::empty()
    }

    /// Decode a subscribed iced event into typed actions to enqueue.
    /// Pure read of state. Returned messages are pushed onto the
    /// runtime's queue *before* phase 1, so iced widgets see the
    /// event too. Default: no follow-ups.
    fn event_process(&self, _event: &iced_core::Event) -> Vec<Self::Message> {
        Vec::new()
    }

    /// Pure derivation of follow-up actions from a message just
    /// applied. Returned messages join the same tick's queue and are
    /// processed FIFO. Default: no follow-ups.
    fn process(&self, _message: &Self::Message) -> Vec<Self::Message> {
        Vec::new()
    }

    /// Theme to render with. Default: Dark. Override if you want Light or
    /// a custom theme.

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

use bitflags::bitflags;

bitflags! {
    /// Which iced event categories a UI wants to receive in its
    /// `event_process` hook. Use `EventFlags::empty()` for no
    /// subscriptions (default), or OR variants together.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct EventFlags: u8 {
        const KEYBOARD     = 1 << 0;
        const MOUSE        = 1 << 1;
        const WINDOW       = 1 << 2;
        const TOUCH        = 1 << 3;
        const INPUT_METHOD = 1 << 4;
        const CLIPBOARD    = 1 << 5;
        const ALL          = Self::KEYBOARD.bits()
                           | Self::MOUSE.bits()
                           | Self::WINDOW.bits()
                           | Self::TOUCH.bits()
                           | Self::INPUT_METHOD.bits()
                           | Self::CLIPBOARD.bits();
    }
}

impl EventFlags {
    /// Returns true if the given event's category is in this flag set.
    pub fn matches(&self, event: &iced_core::Event) -> bool {
        use iced_core::Event;
        match event {
            Event::Keyboard(_) => self.contains(Self::KEYBOARD),
            Event::Mouse(_) => self.contains(Self::MOUSE),
            Event::Window(_) => self.contains(Self::WINDOW),
            Event::Touch(_) => self.contains(Self::TOUCH),
            Event::InputMethod(_) => self.contains(Self::INPUT_METHOD),
            Event::Clipboard(_) => self.contains(Self::CLIPBOARD),
        }
    }
}

/// Opt-in read-back for a UI the compositor inspects synchronously.
///
/// Off-thread the compositor cannot borrow `&U` — the runtime lives on the worker
/// — so a UI that must be read from compositor code publishes a `Send + Clone`
/// copy instead. Only the handful that are actually read implement this; the rest
/// need no change, which is why it is a separate trait rather than an associated
/// type on [`IcedUi`] (associated-type defaults are nightly-only).
///
/// The snapshot is one frame old by construction. That is fine for the reads we
/// have — a form's committed state, a hovered tooltip — and would not be for a
/// read-modify-write, which should become a message instead.
pub trait IcedSnapshot: IcedUi {
    type Snapshot: Send + Sync + Clone + 'static;
    fn snapshot(&self) -> Self::Snapshot;
}

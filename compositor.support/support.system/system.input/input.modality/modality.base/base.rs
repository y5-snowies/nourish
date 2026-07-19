//! The active input modality — which class of physical device produced the input
//! currently being handled.
//!
//! Its own crate (rather than a field on the event enum) because most consumers —
//! the canvas press path, the camera's gesture-ownership test, the selection
//! toolbar, the on-screen keyboard — need only this classification and not the
//! whole `InputEvent` shape.
//!
//! **Why it exists.** Touch and pen do not have their own paths through the
//! compositor: they are translated into *emulated* pointer events by the seat
//! (`touch::emulate`, `tablet::tip`) precisely so that canvas grabs, window focus
//! and viewport routing behave identically for every device. The cost is that a
//! receiver cannot otherwise tell a finger from a mouse click.
//!
//! Behaviour that is only correct for a DIRECT device must therefore gate on
//! [`Modality::is_direct`] — never on a shared tool/grab slot, because the keyboard
//! can arm the very same Select/Hand grab for a mouse user.

/// Device class behind an input event.
///
/// `Trackpad` is distinct from `Mouse` because a trackpad emits its physical button
/// through the *same* device it emits swipes and pinches through, so a button code
/// alone cannot classify it; the seat derives it from the device's gesture
/// capability instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Modality {
    /// A discrete mouse, or any pointer device without gesture support.
    #[default]
    Mouse,
    /// A touchpad — an indirect pointer, but gesture-capable.
    Trackpad,
    /// A touchscreen finger, arriving as an emulated pointer event.
    Touch,
    /// A tablet stylus, arriving as an emulated pointer event.
    Pen,
}

impl Modality {
    /// The user is pointing AT the surface (finger or stylus) rather than driving an
    /// indirect cursor. Touch-specific affordances — the sticky Select frame's grab
    /// handles, placeholder move/scale, hand-tool passthrough — are only correct here,
    /// because they are drawn only for touch and would be invisible to a mouse user.
    pub fn is_direct(self) -> bool {
        matches!(self, Modality::Touch | Modality::Pen)
    }
}

//! Mode enum for [`PlaceholderUi`].

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Compact view: icon + name + Launch / Edit buttons.
    View,
    /// Editor: scrollable attribute form with handler picker.
    Settings,
    /// Confirmation prompt: the plan launches into a container that is not
    /// running, and the user has to say whether to start it first. The
    /// container's name is held in `PlaceholderUi::pending_container`.
    ConfirmContainerStart,
}

impl Default for Mode {
    fn default() -> Self {
        Self::View
    }
}

//! [`PlaceholderMessage`] — public outgoing/incoming messages plus the
//! internal UI-only messages used for widget interactions.
//!
//! The compositor only sees the public variants. The internal ones drive
//! mode switching, field editing, and handler selection inside the UI
//! and never leak out.

use compositor_introspection_launchplan_plan_base::LaunchPlan;

#[derive(Debug, Clone)]
pub enum PlaceholderMessage {
    // ── Outgoing: UI → compositor ─────────────────────────────────

    /// User clicked Launch in View mode.
    LaunchClicked,
    
    DismissClicked,

    /// User clicked Save in Settings mode. The compositor updates its
    /// canonical state and may push back via `UpdatePlan`.
    SaveClicked { updated_plan: Box<LaunchPlan> },
    RestoreClicked,

    /// User confirmed starting the stopped container and launching into it.
    ContainerStartConfirmed,

    /// Adopt the newest sample the compositor delivered for this placeholder
    /// and drop every saved preference. Offered only when a newer sample
    /// exists; see `PlaceholderUi::has_pending_sample`.
    PullSample,

    // ── Incoming: compositor → UI (via dispatch_message) ──────────

    /// Replace the visible plan. Sent by the compositor when the plan
    /// changes externally (e.g., the compositor refreshed extraction).
    UpdatePlan(Box<LaunchPlan>),

    /// Force-switch back to view mode.
    EnterViewMode,

    /// The Launch the user asked for targets a container that is not running.
    /// Raises the confirmation prompt naming `container`; the compositor does
    /// nothing further until the user answers.
    ConfirmContainerStart { container: String },

    // ── Internal: UI-only mode + edit interactions ────────────────

    /// User clicked Edit in View mode.
    EnterSettings,

    /// User clicked Cancel in Settings mode.
    CancelSettings,

    /// User declined starting the container. Returns to View; nothing launches.
    CancelContainerStart,

    /// Active handler choice changed (`None` = no handler synthesis).
    ActiveHandlerChanged(Option<compositor_introspection_extraction_window_base::HandlerId>),

    /// Drop an attribute's override so it falls back to its inferred value.
    /// Distinct from clearing the editor text, which must keep meaning
    /// "override with an empty value".
    AttributeOverrideCleared { descriptor_key: &'static str },

    /// An attribute's enabled flag toggled.
    AttributeEnabledChanged {
        descriptor_key: &'static str,
        enabled: bool,
    },

    /// An attribute's capture flag toggled. When armed, a newly-mapped
    /// window must match this attribute for the placeholder to adopt it
    /// without an explicit Launch.
    AttributeCaptureToggled {
        descriptor_key: &'static str,
        capture: bool,
    },

    /// User typed a new text value for an attribute (Text/Path/EnumOf).
    AttributeTextChanged {
        descriptor_key: &'static str,
        value: String,
    },

    /// User toggled a Bool attribute.
    AttributeBoolChanged {
        descriptor_key: &'static str,
        value: bool,
    },

    /// User changed an item in a StringList attribute.
    AttributeStringListItemChanged {
        descriptor_key: &'static str,
        index: usize,
        value: String,
    },

    /// User added a new (empty) item to a StringList attribute.
    AttributeStringListAdd { descriptor_key: &'static str },

    /// User removed an item from a StringList attribute.
    AttributeStringListRemove {
        descriptor_key: &'static str,
        index: usize,
    },

    /// User changed an env-pair entry. `field` is "key" or "value".
    AttributeEnvPairChanged {
        descriptor_key: &'static str,
        index: usize,
        field: EnvField,
        value: String,
    },

    /// User added a new env pair (empty key, empty value).
    AttributeEnvPairAdd { descriptor_key: &'static str },

    /// User removed an env pair.
    AttributeEnvPairRemove {
        descriptor_key: &'static str,
        index: usize,
    },

    // ── Combo-box (alternatives picker) interactions ──────────────

    /// User opened the alternatives picker for an attribute. The UI
    /// rebuilds combo_state with that attribute's alternatives and
    /// shows it expanded.
    ComboOpen { descriptor_key: &'static str },

    /// The combo_box was dismissed (selection made, focus lost, etc.).
    ComboClose,

    /// User picked an alternative label from the combo_box. The UI
    /// looks up the value by label among the current alternatives and
    /// applies it as the override.
    AlternativeSelected {
        descriptor_key: &'static str,
        label: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvField {
    Key,
    Value,
}

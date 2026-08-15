use compositor_introspection_extraction_window_base as w;
use compositor_introspection_extraction_window_hints_codec::codec::{self, AnyArc, HintCodec};
use std::any::TypeId;
use std::sync::{Arc, Once};

static REGISTERED: Once = Once::new();

/// Register codecs for all built-in attributes (idempotent). Call before any
/// hint persistence/rehydration; cheap after the first call.
pub fn register_standard_codecs() {
    REGISTERED.call_once(|| {
        use w::attributes::*;
        codec::register::<DisplayName>();
        codec::register::<Title>();
        codec::register::<AppId>();
        codec::register::<DesktopEntryPath>();
        codec::register::<IconPath>();
        codec::register::<IconName>();
        codec::register::<XdgIconName>();
        codec::register::<Sandbox>();
        codec::register::<DBusActivatable>();
        codec::register::<DBusServiceName>();
        codec::register::<ExecProgram>();
        codec::register::<ExecArgs>();
        codec::register::<WorkingDirectory>();
        codec::register::<EnvOverlay>();
        {
            use w::handlers::chrome::attributes::*;
            codec::register::<Variant>();
            codec::register::<ProfileDirectory>();
            codec::register::<UserDataDir>();
            codec::register::<AvailableProfiles>();
            codec::register::<ActiveTabTitleGuess>();
            codec::register::<Urls>();
            codec::register::<AppModeUrl>();
            codec::register::<NewWindow>();
            codec::register::<Incognito>();
        }
        {
            use w::handlers::jetbrains::attributes::*;
            codec::register::<ProductAttr>();
            codec::register::<LauncherKindAttr>();
            codec::register::<ProjectNameGuess>();
            codec::register::<ProjectPath>();
        }
        {
            use w::handlers::terminal::attributes::*;
            codec::register::<TerminalKindAttr>();
            codec::register::<LaunchCwd>();
            codec::register::<ForegroundCwd>();
            codec::register::<Shell>();
        }
        codec::register::<w::handlers::nautilus::attributes::LocationUri>();
        register_detected_handler();
    });
}

/// `DetectedHandler`'s value is a `HandlerId` (TypeId-based, not serde) — encode
/// it as a stable handler-name string and resolve it back on load.
fn register_detected_handler() {
    use w::HintAttribute;
    use w::attributes::DetectedHandler;
    codec::register_raw(HintCodec {
        attr_name: DetectedHandler::name(),
        attr_type_id: TypeId::of::<DetectedHandler>(),
        category: DetectedHandler::category(),
        value_type_id: TypeId::of::<w::HandlerId>(),
        to_json: |arc| {
            arc.downcast_ref::<w::HandlerId>().map(|h| serde_json::Value::String(h.name().to_string()))
        },
        from_json: |val| val.as_str().and_then(handler_id_from_name).map(|h| Arc::new(h) as AnyArc),
    });
}

/// Resolve a persisted handler-name string back to its `HandlerId`.
pub fn handler_id_from_name(name: &str) -> Option<w::HandlerId> {
    [
        w::handlers::chrome::id(),
        w::handlers::jetbrains::id(),
        w::handlers::terminal::id(),
        w::handlers::nautilus::id(),
        w::handlers::generic::id(),
    ]
    .into_iter()
    .find(|h| h.name() == name)
}

//! [`PlaceholderUi`] — the iced UI instance.

use std::any::TypeId;
use std::sync::Arc;

use iced_core::{Element, Theme};
use iced_widget::combo_box;
use compositor_introspection_extraction_window_base::{HandlerId, HandlerRegistry, IconPixels};
use compositor_introspection_inference_hint_base::AttributeDescriptor;
use compositor_introspection_launchplan_plan_base::LaunchPlan;
use compositor_support_iced_core_engine_base::{IcedUi, Renderer};

use crate::message::{EnvField, PlaceholderMessage};
use crate::mode::Mode;
use crate::view;

/// The placeholder iced UI.
///
/// Holds:
/// - `canonical`: the last [`LaunchPlan`] received from the compositor.
/// - `working`: the live editing copy used while in Settings mode.
/// - `mode`: which view to render.
/// - `registry`: shared handler registry from the compositor.
/// - `combo_active`: which attribute's combo_box is currently expanded
///   (at most one). `None` means all alternatives pickers render in a
///   collapsed state.
/// - `combo_state`: iced combo_box state for the currently-active combo.
///   Rebuilt when the active attribute changes.
pub struct PlaceholderUi {
    pub(crate) canonical: LaunchPlan,
    pub(crate) session: Option<LaunchPlan>,
    /// Whether the captured window's client declared an
    /// `xdg-session-management-v1` identity.
    ///
    /// NOT `session` above, which is the pending-sample plan and unrelated
    /// despite the name. This is what makes the placeholder restorable by identity
    /// rather than by guesswork, which is worth showing.
    pub(crate) has_session_identity: bool,
    /// Whether the captured window was an X11 (XWayland) client. Display only —
    /// it warms the panel background so an X11 placeholder is recognisable at a
    /// glance, which matters because X11 placeholders restore by a different route
    /// and are the first thing to check when a restore misses.
    pub(crate) from_x11: bool,
    /// The icon the captured window committed for itself, if it had one.
    ///
    /// Preferred over the plan's `IconPath` when present — the same order
    /// `overview.draw/draw.icon` already applies, and for the reason its doc gives:
    /// the protocol icon is live and window-specific, while the entry is an app-level
    /// lookup keyed on `app_id`. Two surfaces showing the same window must not
    /// disagree about its icon.
    ///
    /// Session-only, so a restored placeholder has `None` here and falls back to the
    /// entry — see `Placeholder::icon_pixels`.
    pub(crate) icon_pixels: Option<IconPixels>,
    pub(crate) working: LaunchPlan,
    pub(crate) mode: Mode,
    pub(crate) registry: Arc<HandlerRegistry>,
    pub(crate) combo_active: Option<&'static str>,
    pub(crate) combo_state: combo_box::State<String>,
    /// Container named by the pending [`Mode::ConfirmContainerStart`] prompt.
    pub(crate) pending_container: Option<String>,
    /// Attributes whose inferred value was refreshed because something they
    /// were derived from changed: attribute key -> the source that changed.
    ///
    /// UI state, not plan state — it explains a change the user just caused
    /// and has no meaning once the working copy is replaced. So it is dropped
    /// wholesale by Cancel / Discard / Pull rather than persisted.
    pub(crate) refreshed: Vec<(&'static str, &'static str, bool)>,
}

impl PlaceholderUi {
    pub fn new(
        plan: LaunchPlan,
        plan_session: Option<LaunchPlan>,
        has_session_identity: bool,
        from_x11: bool,
        icon_pixels: Option<IconPixels>,
        registry: Arc<HandlerRegistry>,
    ) -> Self {
        Self {
            working: plan.clone(),
            canonical: plan,
            session: plan_session,
            has_session_identity,
            from_x11,
            icon_pixels,
            mode: Mode::View,
            registry,
            combo_active: None,
            combo_state: combo_box::State::new(Vec::new()),
            pending_container: None,
            refreshed: Vec::new(),
        }
    }

    /// Whether the compositor has delivered a newer sample than the saved
    /// plan — i.e. this placeholder was restored into a live window at some
    /// point and the sampler re-extracted it. Only then is there anything to
    /// pull, so the button is offered only then.
    pub fn has_pending_sample(&self) -> bool {
        self.session.is_some()
    }

    /// The currently-edited plan in Settings mode, or the canonical
    /// plan in View mode. Handy for rendering: both modes read the
    /// effective values via this accessor.
    pub fn shown_plan(&self) -> &LaunchPlan {
        match self.mode {
            Mode::Settings => &self.working,
            // The confirmation prompt is a modal over the View, so it reads the
            // same canonical plan the placeholder behind it does.
            Mode::View | Mode::ConfirmContainerStart => &self.canonical,
        }
    }
}

impl IcedUi for PlaceholderUi {
    type Message = PlaceholderMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            PlaceholderMessage::LaunchClicked => {
                // Compositor-handled. Do nothing in the UI.
            }
            PlaceholderMessage::DismissClicked => {
                // Compositor-handled. Do nothing in the UI.
            }
            
            // "Discard changes": throw away the edits made since entering
            // Settings and go back to what is SAVED. It used to prefer the
            // session plan when one existed, which made it look like a
            // re-pull that only ever restored your own saved edits — pulling
            // a fresh sample is `PullSample`, and the two are now distinct.
            PlaceholderMessage::RestoreClicked { } => {
                self.working = self.canonical.clone();
                self.refreshed.clear();
            }

            // "Pull latest": adopt the newest sample and drop every saved
            // preference. Built with `LaunchPlan::new`, so it carries the
            // sample's inferred hints and NO overrides — that is what makes
            // a deleted env var (or any other edit) come back.
            PlaceholderMessage::PullSample => {
                if let Some(session) = &self.session {
                    self.working = LaunchPlan::new(session.application_data.clone());
                }
                // A pulled plan's hints all came from one extraction, so
                // nothing is derived-from-something-stale; and the overrides
                // that could have conflicted are gone by definition. Clearing
                // the tips is the whole of "act on invalidate" here.
                self.refreshed.clear();
            }
            
            PlaceholderMessage::SaveClicked { .. } => {
                // Compositor-handled. UI doesn't synthesize this itself —
                // it's emitted only from the Save button handler, which
                // is in the settings view. After Save the compositor will
                // push an UpdatePlan and EnterViewMode to bring us back.
            }

            PlaceholderMessage::UpdatePlan(plan) => {
                let plan = *plan;
                // Don't trample the user's working copy if they're
                // currently editing — they may have unsaved changes.
                if self.mode == Mode::View {
                    self.working = plan.clone();
                }
                self.canonical = plan;
                self.mode = Mode::View;
            }
            PlaceholderMessage::EnterViewMode => {
                self.mode = Mode::View;
                self.working = self.canonical.clone();
                self.pending_container = None;
            }

            PlaceholderMessage::ConfirmContainerStart { container } => {
                self.pending_container = Some(container);
                self.mode = Mode::ConfirmContainerStart;
            }
            PlaceholderMessage::CancelContainerStart => {
                self.pending_container = None;
                self.mode = Mode::View;
            }
            PlaceholderMessage::ContainerStartConfirmed => {
                // Compositor-handled. Drop the prompt here so the placeholder is back
                // to its normal face while the container starts.
                self.pending_container = None;
                self.mode = Mode::View;
            }

            PlaceholderMessage::EnterSettings => {
                self.working = self.canonical.clone();
                self.mode = Mode::Settings;
                self.refreshed.clear();
            }
            PlaceholderMessage::CancelSettings => {
                self.working = self.canonical.clone();
                self.mode = Mode::View;
                self.refreshed.clear();
            }
            PlaceholderMessage::ActiveHandlerChanged(handler) => {
                self.working.set_active_handler(handler);
            }
            PlaceholderMessage::AttributeOverrideCleared { descriptor_key } => {
                if let Some(d) = self.descriptor_by_key(descriptor_key) {
                    self.working.clear_pref_override(&d);
                }
                self.invalidate_dependents(descriptor_key);
            }
            PlaceholderMessage::AttributeEnabledChanged {
                descriptor_key,
                enabled,
            } => {
                if let Some(d) = self.descriptor_by_key(descriptor_key) {
                    self.working.set_enabled_raw(&d, enabled);
                }
                self.invalidate_dependents(descriptor_key);
            }
            PlaceholderMessage::AttributeCaptureToggled {
                descriptor_key,
                capture,
            } => {
                if let Some(d) = self.descriptor_by_key(descriptor_key) {
                    compositor_introspection_launchplan_plan_capture::capture::set_capture_raw(&mut self.working, &d, capture);
                }
            }
            PlaceholderMessage::AttributeTextChanged {
                descriptor_key,
                value,
            } => {
                self.apply_text_change(descriptor_key, value);
                self.invalidate_dependents(descriptor_key);
            }
            PlaceholderMessage::AttributeBoolChanged {
                descriptor_key,
                value,
            } => {
                self.apply_bool_change(descriptor_key, value);
                self.invalidate_dependents(descriptor_key);
            }
            PlaceholderMessage::AttributeStringListItemChanged {
                descriptor_key,
                index,
                value,
            } => {
                self.mutate_string_list(descriptor_key, |list| {
                    if let Some(slot) = list.get_mut(index) {
                        *slot = value;
                    }
                });
            }
            PlaceholderMessage::AttributeStringListAdd { descriptor_key } => {
                self.mutate_string_list(descriptor_key, |list| {
                    list.push(String::new());
                });
            }
            PlaceholderMessage::AttributeStringListRemove {
                descriptor_key,
                index,
            } => {
                self.mutate_string_list(descriptor_key, |list| {
                    if index < list.len() {
                        list.remove(index);
                    }
                });
            }
            PlaceholderMessage::AttributeEnvPairChanged {
                descriptor_key,
                index,
                field,
                value,
            } => {
                self.mutate_env_pair_list(descriptor_key, |list| {
                    if let Some(pair) = list.get_mut(index) {
                        match field {
                            EnvField::Key => pair.key = value,
                            EnvField::Value => pair.value = value,
                        }
                    }
                });
            }
            PlaceholderMessage::AttributeEnvPairAdd { descriptor_key } => {
                self.mutate_env_pair_list(descriptor_key, |list| {
                    list.push(compositor_introspection_extraction_window_base::EnvPair {
                        key: String::new(),
                        value: String::new(),
                    });
                });
            }
            PlaceholderMessage::AttributeEnvPairRemove {
                descriptor_key,
                index,
            } => {
                self.mutate_env_pair_list(descriptor_key, |list| {
                    if index < list.len() {
                        list.remove(index);
                    }
                });
            }

            PlaceholderMessage::ComboOpen { descriptor_key } => {
                self.open_combo(descriptor_key);
            }
            PlaceholderMessage::ComboClose => {
                self.combo_active = None;
                self.combo_state = combo_box::State::new(Vec::new());
            }
            PlaceholderMessage::AlternativeSelected {
                descriptor_key,
                label,
            } => {
                self.apply_alternative(descriptor_key, &label);
                self.invalidate_dependents(descriptor_key);
                self.combo_active = None;
                self.combo_state = combo_box::State::new(Vec::new());
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Message, Theme, Renderer> {
        view::root_view(self)
    }
}

// ── Internal helpers ─────────────────────────────────────────────────

impl PlaceholderUi {
    /// Find a descriptor by its key string among all descriptors that
    /// apply to the working plan.
    pub(crate) fn descriptor_by_key(&self, key: &str) -> Option<AttributeDescriptor> {
        self.all_descriptors().into_iter().find(|d| d.key == key)
    }

    /// All descriptors visible in the settings UI for the working plan.
    /// Identity + Launch + active handler's scoped attributes.
    pub(crate) fn all_descriptors(&self) -> Vec<AttributeDescriptor> {
        compositor_introspection_inference_hint_base::all_descriptors_for(
            &self.registry,
            self.working.active_handler,
        )
    }

    /// Refresh whatever was derived from `key`, and remember what moved so
    /// the editor can explain it. Automatic rather than offered: the refreshed
    /// values are the true ones, and leaving stale derivations on screen while
    /// asking permission to fix them would be the worse trade.
    fn invalidate_dependents(&mut self, key: &'static str) {
        let registry = self.registry.clone();
        let refreshed = compositor_introspection_launchplan_plan_invalidate::invalidate::on_changed(
            &mut self.working,
            key,
            &registry,
        );
        for item in refreshed {
            self.refreshed.retain(|(attr, _, _)| *attr != item.attribute);
            self.refreshed.push((item.attribute, key, item.recomputed));
        }
    }

    /// What to tell the user about `key`, if anything refreshed it.
    pub(crate) fn refresh_note(&self, key: &str) -> Option<String> {
        let (_, source, recomputed) = self.refreshed.iter().find(|(attr, _, _)| *attr == key)?;
        Some(if *recomputed {
            format!("updated — re-derived after {source} changed")
        } else {
            format!("may be out of date — {source} changed and this cannot be re-derived")
        })
    }

    fn apply_text_change(&mut self, key: &str, value: String) {
        let Some(d) = self.descriptor_by_key(key) else { return };
        use compositor_introspection_extraction_window_base::AttributeKind as K;
        match &d.kind {
            K::Text => {
                self.working.set_pref_raw(
                    &d,
                    Arc::new(value),
                    TypeId::of::<String>(),
                );
            }
            // An EnumOf attribute's value is its OWN type, not a String —
            // `terminal.kind` is a `TerminalKind`. Writing the variant name as
            // a `String` stored an override whose TypeId did not match the
            // attribute, so `Preferences::get::<A>()` rejected it on the type
            // check and the launch silently ignored it, while `get_raw` (which
            // does not check) happily showed it as applied in the editor. An
            // override that looks applied and isn't is worse than none.
            //
            // Decode through the codec registry — the same one persistence
            // uses to rebuild typed overrides — so the stored value carries
            // the attribute's real type.
            K::EnumOf(_) => {
                compositor_introspection_extraction_window_hints_codec_register::register::register_standard_codecs();
                let json = serde_json::Value::String(value);
                let decoded = compositor_introspection_extraction_window_hints_codec::codec::decode(d.key, &json);
                let type_id = compositor_introspection_extraction_window_hints_codec::codec::value_type_id(d.key);
                if let (Some(arc), Some(type_id)) = (decoded, type_id) {
                    self.working.set_pref_raw(&d, arc, type_id);
                }
            }
            K::Path => {
                use std::path::PathBuf;
                self.working.set_pref_raw(
                    &d,
                    Arc::new(PathBuf::from(value)),
                    TypeId::of::<PathBuf>(),
                );
            }
            K::Custom(tag) if *tag == "chrome_profile" => {
                self.working.set_pref_raw(
                    &d,
                    Arc::new(value),
                    TypeId::of::<String>(),
                );
            }
            // Other custom kinds (sandbox, handler_id, profile_list,
            // env_pair_list) don't accept Text input.
            _ => {}
        }
    }

    fn apply_bool_change(&mut self, key: &str, value: bool) {
        let Some(d) = self.descriptor_by_key(key) else { return };
        self.working
            .set_pref_raw(&d, Arc::new(value), TypeId::of::<bool>());
    }

    fn mutate_string_list<F: FnOnce(&mut Vec<String>)>(&mut self, key: &str, f: F) {
        let Some(d) = self.descriptor_by_key(key) else { return };
        let mut current: Vec<String> = self
            .working
            .current_raw(&d)
            .and_then(|a| a.downcast_ref::<Vec<String>>().cloned())
            .unwrap_or_default();
        f(&mut current);
        self.working.set_pref_raw(
            &d,
            Arc::new(current),
            TypeId::of::<Vec<String>>(),
        );
    }

    fn mutate_env_pair_list<F>(&mut self, key: &str, f: F)
    where
        F: FnOnce(&mut Vec<compositor_introspection_extraction_window_base::EnvPair>),
    {
        let Some(d) = self.descriptor_by_key(key) else { return };
        let mut current: Vec<compositor_introspection_extraction_window_base::EnvPair> = self
            .working
            .current_raw(&d)
            .and_then(|a| {
                a.downcast_ref::<Vec<compositor_introspection_extraction_window_base::EnvPair>>()
                    .cloned()
            })
            .unwrap_or_default();
        f(&mut current);
        self.working.set_pref_raw(
            &d,
            Arc::new(current),
            TypeId::of::<Vec<compositor_introspection_extraction_window_base::EnvPair>>(),
        );
    }

    /// List of handlers the user can switch to via the picker.
    /// Returns id + display name pairs in deterministic order.
    pub(crate) fn handler_choices(&self) -> Vec<(HandlerId, String)> {
        let mut ids: Vec<HandlerId> = self.registry.ids().collect();
        ids.sort_by_key(|id| id.name());
        ids.into_iter()
            .map(|id| (id, id.to_string()))
            .collect()
    }

    // ── Alternatives picker (combo_box) ──────────────────────────

    /// Display-string labels for the alternatives of one attribute, in
    /// the same order [`alternatives_for`] returns them. Used to
    /// populate the combo_box options list.
    ///
    /// Each label includes a summary of the value, its source, and
    /// confidence so the user can disambiguate.
    pub(crate) fn alternative_labels(&self, descriptor: &AttributeDescriptor) -> Vec<String> {
        self.alternatives_for(descriptor)
            .iter()
            .map(|alt| {
                let base = format!(
                    "{}  ·  {:?}  ·  {}",
                    crate::view::settings::attribute_widget::summarize_value(&alt.raw.value),
                    alt.raw.source.method,
                    confidence_label(alt.raw.confidence),
                );
                // Mark the era. Without it the list silently mixes what was
                // saved with what has since been observed, and picking one
                // would be a guess about which you were getting.
                if alt.from_sample {
                    format!("{base}  ·  latest sample")
                } else {
                    base
                }
            })
            .collect()
    }

    /// Every candidate value offered for an attribute: those in the SAVED
    /// plan, plus those the newest sample found.
    ///
    /// Merging the sample in gives per-attribute granularity that "Pull
    /// latest" cannot — adopt one freshly-observed value while keeping every
    /// other override — and it makes the picker appear at all for an
    /// attribute that has no saved alternatives but does have a new one.
    ///
    /// Deduplicated by VALUE, saved entries first, so a value present in both
    /// appears once and keeps its saved provenance. Identity is the codec's
    /// JSON encoding (the same value-equality trick restoration matching
    /// uses), falling back to the rendered summary for an attribute with no
    /// registered codec — two entries that would read identically in the
    /// picker are duplicates as far as the user is concerned.
    pub(crate) fn alternatives_for(&self, descriptor: &AttributeDescriptor) -> Vec<Alternative> {
        compositor_introspection_extraction_window_hints_codec_register::register::register_standard_codecs();
        let key = descriptor.key;

        let saved = self.working.application_data.hints.available_raw(key);
        let sampled = self
            .session
            .as_ref()
            .map(|s| s.application_data.hints.available_raw(key))
            .unwrap_or_default();

        let mut out: Vec<Alternative> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let tagged = saved
            .into_iter()
            .map(|raw| (raw, false))
            .chain(sampled.into_iter().map(|raw| (raw, true)));

        for (raw, from_sample) in tagged {
            let identity = alternative_identity(key, &raw);
            if seen.contains(&identity) {
                continue;
            }
            seen.push(identity);
            out.push(Alternative { raw, from_sample });
        }
        out
    }

    fn open_combo(&mut self, key: &'static str) {
        let Some(d) = self.descriptor_by_key(key) else { return };
        let labels = self.alternative_labels(&d);
        self.combo_state = combo_box::State::new(labels);
        self.combo_active = Some(key);
    }

    fn apply_alternative(&mut self, key: &str, chosen_label: &str) {
        let Some(d) = self.descriptor_by_key(key) else { return };
        let alternatives = self.alternatives_for(&d);
        let labels = self.alternative_labels(&d);
        let Some(index) = labels.iter().position(|l| l == chosen_label) else { return };
        let alt = &alternatives[index];
        let type_id = (*alt.raw.value).type_id();
        self.working.set_pref_raw(&d, alt.raw.value.clone(), type_id);
    }
}

/// One candidate value in an attribute's picker, and which era it came from.
pub(crate) struct Alternative {
    pub(crate) raw: compositor_introspection_extraction_window_base::RawAlternative,
    /// From the newest sample rather than the saved plan.
    pub(crate) from_sample: bool,
}

/// Dedup key for an alternative: the codec's JSON encoding where the
/// attribute has one, else the summary the picker would render. Never the
/// `Arc` pointer — the same value observed in two samples is two allocations
/// and would not dedup.
fn alternative_identity(
    key: &str,
    alt: &compositor_introspection_extraction_window_base::RawAlternative,
) -> String {
    compositor_introspection_extraction_window_hints_codec::codec::encode(key, &alt.value)
        .map(|json| json.to_string())
        .unwrap_or_else(|| crate::view::settings::attribute_widget::summarize_value(&alt.value))
}

fn confidence_label(c: compositor_introspection_extraction_window_base::Confidence) -> &'static str {
    use compositor_introspection_extraction_window_base::Confidence;
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

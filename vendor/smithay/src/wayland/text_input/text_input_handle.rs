use std::mem;
use std::sync::{Arc, Mutex};

use tracing::debug;
use wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
    self, ChangeCause, ContentHint, ContentPurpose, ZwpTextInputV3,
};
use wayland_server::backend::{ClientId, ObjectId};
use wayland_server::{Resource, protocol::wl_surface::WlSurface};

use crate::input::SeatHandler;
use crate::utils::{Logical, Rectangle};
use crate::wayland::{Dispatch2, input_method::InputMethodHandle};

#[derive(Default, Debug)]
pub(crate) struct TextInput {
    instances: Vec<Instance>,
    focus: Option<WlSurface>,
    active_text_input_id: Option<ObjectId>,
    compositor_input_method: bool,
    /// y5: latest `set_surrounding_text` (text, cursor, anchor) from the active
    /// client, so the on-screen keyboard can show a live preview of the field.
    /// Cleared on leave / disable.
    surrounding_text: Option<(String, u32, u32)>,
    /// y5: the active field is a password / PIN / sensitive-data field (from the
    /// client's `set_content_type`). The OSK preview is suppressed for these so a
    /// typed secret is never rendered on screen.
    content_sensitive: bool,
    /// y5: bumped on every field transition (enable / disable / focus leave), so the OSK
    /// can detect a switch between two fields that share ONE surface (e.g. two inputs on
    /// a web page, where the `WlSurface` never changes) and clear its local echo.
    field_generation: u64,
}

impl TextInput {
    fn with_focused_client_all_text_inputs<F>(&mut self, mut f: F)
    where
        F: FnMut(&ZwpTextInputV3, &WlSurface, u32),
    {
        if let Some(surface) = self.focus.as_ref().filter(|surface| surface.is_alive()) {
            for text_input in self.instances.iter() {
                let instance_id = text_input.instance.id();
                if instance_id.same_client_as(&surface.id()) {
                    f(&text_input.instance, surface, text_input.serial);
                }
            }
        };
    }

    fn with_active_text_input<F>(&mut self, mut f: F)
    where
        F: FnMut(&ZwpTextInputV3, &WlSurface, u32),
    {
        let active_id = match &self.active_text_input_id {
            Some(active_text_input_id) => active_text_input_id,
            None => return,
        };

        let surface = match self.focus.as_ref().filter(|surface| surface.is_alive()) {
            Some(surface) => surface,
            None => return,
        };

        let surface_id = surface.id();
        if let Some(text_input) = self
            .instances
            .iter()
            .filter(|instance| instance.instance.id().same_client_as(&surface_id))
            .find(|instance| &instance.instance.id() == active_id)
        {
            f(&text_input.instance, surface, text_input.serial);
        }
    }
}

/// Handle to text input instances
#[derive(Default, Debug, Clone)]
pub struct TextInputHandle {
    pub(crate) inner: Arc<Mutex<TextInput>>,
}

impl TextInputHandle {
    pub(super) fn add_instance(&self, instance: &ZwpTextInputV3) {
        let mut inner = self.inner.lock().unwrap();
        inner.instances.push(Instance {
            instance: instance.clone(),
            serial: 0,
            pending_state: Default::default(),
        });
    }

    fn increment_serial(&self, text_input: &ZwpTextInputV3) {
        if let Some(instance) = self
            .inner
            .lock()
            .unwrap()
            .instances
            .iter_mut()
            .find(|instance| instance.instance == *text_input)
        {
            instance.serial += 1
        }
    }

    /// Return the currently focused surface.
    pub fn focus(&self) -> Option<WlSurface> {
        self.inner.lock().unwrap().focus.clone()
    }

    /// Advance the focus for the client to `surface`.
    ///
    /// This doesn't send any 'enter' or 'leave' events.
    pub fn set_focus(&self, surface: Option<WlSurface>) {
        self.inner.lock().unwrap().focus = surface;
    }

    /// Send `leave` on the text-input instance for the currently focused
    /// surface.
    pub fn leave(&self) {
        let mut inner = self.inner.lock().unwrap();
        // Leaving clears the active text input.
        inner.active_text_input_id = None;
        // y5: the preview is field-scoped — drop it when focus leaves.
        inner.surrounding_text = None;
        inner.content_sensitive = false;
        inner.field_generation = inner.field_generation.wrapping_add(1);
        // NOTE: we implement it in a symmetrical way with `enter`.
        inner.with_focused_client_all_text_inputs(|text_input, focus, _| {
            text_input.leave(focus);
        });
    }

    /// y5: the latest surrounding text `(text, cursor, anchor)` reported by the
    /// active text-input client, for the on-screen keyboard's preview line. `None`
    /// when no field is active, the client never reports surrounding text, or the
    /// field is sensitive (password/PIN) — a secret is never surfaced for preview.
    pub fn surrounding_text(&self) -> Option<(String, u32, u32)> {
        let inner = self.inner.lock().unwrap();
        if inner.content_sensitive {
            return None;
        }
        inner.surrounding_text.clone()
    }

    /// y5: store (or clear) the active field's surrounding text.
    pub(crate) fn set_surrounding_text(&self, value: Option<(String, u32, u32)>) {
        self.inner.lock().unwrap().surrounding_text = value;
    }

    /// y5: mark whether the active field is sensitive (suppresses the preview).
    pub(crate) fn set_content_sensitive(&self, sensitive: bool) {
        self.inner.lock().unwrap().content_sensitive = sensitive;
    }

    /// y5: whether the active field is sensitive (password/PIN) — the OSK never
    /// previews such a field, from either surrounding text OR its own local echo.
    pub fn is_sensitive_field(&self) -> bool {
        self.inner.lock().unwrap().content_sensitive
    }

    /// y5: a counter bumped on each field transition (enable/disable/leave). The OSK
    /// clears its local echo when this changes, so a switch between two fields on the
    /// SAME surface (which never changes the focused `WlSurface`) is still detected.
    pub fn field_generation(&self) -> u64 {
        self.inner.lock().unwrap().field_generation
    }

    /// y5: drop the cached preview for a field that is being replaced or torn down and
    /// mark the transition — in ONE lock acquisition, so a concurrent reader can never
    /// observe a half-reset field (cleared text but stale sensitivity, or vice versa).
    ///
    /// Deliberately does NOT clear `content_sensitive`. A client that re-`enable`s a
    /// field (Chromium does this on focus) may send its surrounding text BEFORE it
    /// re-sends `set_content_type`; clearing the flag here would open a window in which
    /// a password is previewable. The classification is therefore carried over until
    /// the client states otherwise, and is cleared only on `leave`, where no field is
    /// left to protect.
    pub(crate) fn reset_field(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.surrounding_text = None;
        inner.field_generation = inner.field_generation.wrapping_add(1);
    }

    /// y5: send the catch-up `enter` to ONE freshly created instance, if it belongs to
    /// the focused client.
    ///
    /// [`Self::enter`] targets *every* instance the focused client owns, so calling it
    /// from `get_text_input` re-sends `enter` to instances that already received one
    /// with no intervening `leave` — a protocol violation that repeats each time the
    /// client creates another text-input object. Upstream only hit this when an IME was
    /// bound; y5 relaxed that gate, which made it routine.
    pub fn enter_instance(&self, instance: &ZwpTextInputV3) {
        let inner = self.inner.lock().unwrap();
        if let Some(surface) = inner.focus.as_ref().filter(|surface| surface.is_alive()) {
            if instance.id().same_client_as(&surface.id()) {
                instance.enter(surface);
            }
        }
    }

    /// Send `enter` on the text-input instance for the currently focused
    /// surface.
    pub fn enter(&self) {
        let mut inner = self.inner.lock().unwrap();
        // NOTE: protocol states that if we have multiple text inputs enabled, `enter` must
        // be send for each of them.
        inner.with_focused_client_all_text_inputs(|text_input, focus, _| {
            text_input.enter(focus);
        });
    }

    /// Have the compositor act as the input method for this seat.
    ///
    /// While enabled, `enter` is delivered to the focused text-input even when no real
    /// `zwp_input_method_v2` is bound, so the compositor can `commit_string` into it (e.g. for
    /// remote-desktop text injection). Toggling this sends `enter`/`leave` for the current
    /// focus immediately; subsequent focus changes are handled by the keyboard focus logic.
    pub fn set_compositor_input_method(&self, active: bool) {
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.compositor_input_method == active {
                return;
            }
            inner.compositor_input_method = active;
        }
        if active {
            self.enter();
        } else {
            self.leave();
        }
    }

    /// Whether the compositor is currently acting as the input method for this seat
    /// (see [`set_compositor_input_method`](Self::set_compositor_input_method)).
    pub fn compositor_input_method(&self) -> bool {
        self.inner.lock().unwrap().compositor_input_method
    }

    /// The `discard_state` is used when the input-method signaled that
    /// the state should be discarded and wrong serial sent.
    pub fn done(&self, discard_state: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.with_active_text_input(|text_input, _, serial| {
            if discard_state {
                debug!("discarding text-input state due to serial");
                // Discarding is done by sending non-matching serial.
                text_input.done(0);
            } else {
                text_input.done(serial);
            }
        });
    }

    /// Access the text-input instances for the currently focused surface.
    pub fn with_focused_text_input<F>(&self, mut f: F)
    where
        F: FnMut(&ZwpTextInputV3, &WlSurface),
    {
        let mut inner = self.inner.lock().unwrap();
        inner.with_focused_client_all_text_inputs(|ti, surface, _| {
            f(ti, surface);
        });
    }

    /// Access the active text-input instance for the currently focused surface.
    pub fn with_active_text_input<F>(&self, mut f: F)
    where
        F: FnMut(&ZwpTextInputV3, &WlSurface),
    {
        let mut inner = self.inner.lock().unwrap();
        inner.with_active_text_input(|ti, surface, _| {
            f(ti, surface);
        });
    }

    /// Call the callback with the serial of the active text_input or with the passed
    /// `default` one when empty.
    pub(crate) fn active_text_input_serial_or_default<F>(&self, default: u32, mut callback: F)
    where
        F: FnMut(u32),
    {
        let mut inner = self.inner.lock().unwrap();
        let mut should_default = true;
        inner.with_active_text_input(|_, _, serial| {
            should_default = false;
            callback(serial);
        });
        if should_default {
            callback(default)
        }
    }
}

/// User data of ZwpTextInputV3 object
#[derive(Debug)]
pub struct TextInputUserData {
    pub(super) handle: TextInputHandle,
    pub(crate) input_method_handle: InputMethodHandle,
}

impl<D> Dispatch2<ZwpTextInputV3, D> for TextInputUserData
where
    D: SeatHandler,
    D: 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &wayland_server::Client,
        resource: &ZwpTextInputV3,
        request: zwp_text_input_v3::Request,
        _dhandle: &wayland_server::DisplayHandle,
        _data_init: &mut wayland_server::DataInit<'_, D>,
    ) {
        // Always increment serial to not desync with clients.
        if matches!(request, zwp_text_input_v3::Request::Commit) {
            self.handle.increment_serial(resource);
        }

        // y5: upstream discards text-input requests when no IME instance is bound
        // (with an escape hatch for the compositor acting as the input method itself).
        // y5's on-screen keyboard needs the text-input state tracked even without an
        // external IME: processing `enable`/`commit` here is what sets
        // `active_text_input_id`, which `with_active_text_input` reports as the OSK's
        // "a text field is focused" signal. Every input-method forwarding call below
        // (`with_instance`/`activate_input_method`/`set_text_input_rectangle`) is an
        // internal no-op when no instance exists, so proceeding is safe; the client
        // simply receives no preedit/commit and types via the normal keyboard path
        // (into which the OSK injects). When an IME IS bound this is unchanged.

        let focus = match self.handle.focus() {
            Some(focus) if focus.id().same_client_as(&resource.id()) => focus,
            _ => {
                debug!("discarding text-input request for unfocused client");
                return;
            }
        };

        let mut guard = self.handle.inner.lock().unwrap();
        let pending_state = match guard.instances.iter_mut().find_map(|instance| {
            if instance.instance == *resource {
                Some(&mut instance.pending_state)
            } else {
                None
            }
        }) {
            Some(pending_state) => pending_state,
            None => {
                debug!("got request for untracked text-input");
                return;
            }
        };

        match request {
            zwp_text_input_v3::Request::Enable => {
                pending_state.enable = Some(true);
            }
            zwp_text_input_v3::Request::Disable => {
                pending_state.enable = Some(false);
            }
            zwp_text_input_v3::Request::SetSurroundingText { text, cursor, anchor } => {
                pending_state.surrounding_text = Some((text, cursor as u32, anchor as u32));
            }
            zwp_text_input_v3::Request::SetTextChangeCause { cause } => {
                // Guard against clients sending us unknown values from future versions.
                let cause = cause.into_result().unwrap_or(ChangeCause::Other);
                pending_state.text_change_cause = Some(cause);
            }
            zwp_text_input_v3::Request::SetContentType { hint, purpose } => {
                // Guard against clients sending us unknown values from future versions.
                let hint = ContentHint::from_bits_truncate(u32::from(hint));
                let purpose = purpose.into_result().unwrap_or(ContentPurpose::Normal);
                pending_state.content_type = Some((hint, purpose));
            }
            zwp_text_input_v3::Request::SetCursorRectangle { x, y, width, height } => {
                pending_state.cursor_rectangle = Some(Rectangle::new((x, y).into(), (width, height).into()));
            }
            zwp_text_input_v3::Request::Commit => {
                let mut new_state = mem::take(pending_state);
                let _ = pending_state;
                let active_text_input_id = &mut guard.active_text_input_id;

                if active_text_input_id.is_some() && *active_text_input_id != Some(resource.id()) {
                    debug!("discarding text_input request since we already have an active one");
                    return;
                }

                match new_state.enable {
                    Some(true) => {
                        *active_text_input_id = Some(resource.id());
                        // Drop the guard before calling to other subsystem.
                        drop(guard);
                        // y5: fresh field — drop any stale preview and bump the generation
                        // so the OSK clears its echo. Sensitivity is CARRIED OVER rather
                        // than cleared (see `reset_field`): this client may send its
                        // surrounding text before re-stating its content type.
                        self.handle.reset_field();
                        self.input_method_handle.activate_input_method(state, &focus);
                    }
                    Some(false) => {
                        *active_text_input_id = None;
                        // Drop the guard before calling to other subsystem.
                        drop(guard);
                        // y5: field disabled — clear the OSK preview and bump the
                        // generation so the echo clears for the next field.
                        self.handle.reset_field();
                        self.input_method_handle.deactivate_input_method(state);
                        return;
                    }
                    None => {
                        if *active_text_input_id != Some(resource.id()) {
                            debug!("discarding text_input requests before enabling it");
                            return;
                        }

                        // Drop the guard before calling to other subsystems later on.
                        drop(guard);
                    }
                }

                if let Some((text, cursor, anchor)) = new_state.surrounding_text.take() {
                    // y5: keep a copy for the OSK preview, then forward to the IME (no-op
                    // without an instance).
                    self.handle.set_surrounding_text(Some((text.clone(), cursor, anchor)));
                    self.input_method_handle.with_instance(move |input_method| {
                        input_method.object.surrounding_text(text, cursor, anchor)
                    });
                }

                if let Some(cause) = new_state.text_change_cause.take() {
                    self.input_method_handle.with_instance(move |input_method| {
                        input_method.object.text_change_cause(cause);
                    });
                }

                if let Some((hint, purpose)) = new_state.content_type.take() {
                    // y5: suppress the OSK preview for password / PIN / sensitive fields.
                    // HIDDEN_TEXT counts too: toolkits and web engines routinely mark a
                    // password field with `hidden_text` and leave `purpose` as Normal, so
                    // checking SENSITIVE_DATA alone would echo the secret to screen.
                    let sensitive = matches!(purpose, ContentPurpose::Password | ContentPurpose::Pin)
                        || hint.intersects(ContentHint::SensitiveData | ContentHint::HiddenText);
                    self.handle.set_content_sensitive(sensitive);
                    self.input_method_handle.with_instance(move |input_method| {
                        input_method.object.content_type(hint, purpose);
                    });
                }

                if let Some(rect) = new_state.cursor_rectangle.take() {
                    self.input_method_handle
                        .set_text_input_rectangle::<D>(state, rect);
                }

                self.input_method_handle.with_instance(|input_method| {
                    input_method.done();
                });
            }
            zwp_text_input_v3::Request::Destroy => {
                // Nothing to do
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, text_input: &ZwpTextInputV3) {
        let destroyed_id = text_input.id();
        let deactivate_im = {
            let mut inner = self.handle.inner.lock().unwrap();
            inner.instances.retain(|inst| inst.instance.id() != destroyed_id);
            let destroyed_focused = inner
                .focus
                .as_ref()
                .map(|focus| focus.id().same_client_as(&destroyed_id))
                .unwrap_or(true);

            // Deactivate IM when we either lost focus entirely or destroyed text-input for the
            // currently focused client.
            destroyed_focused
                && !inner
                    .instances
                    .iter()
                    .any(|inst| inst.instance.id().same_client_as(&destroyed_id))
        };

        if deactivate_im {
            self.input_method_handle.deactivate_input_method(state);
        }
    }
}

#[derive(Debug)]
struct Instance {
    instance: ZwpTextInputV3,
    serial: u32,
    pending_state: TextInputState,
}

#[derive(Debug, Default)]
struct TextInputState {
    enable: Option<bool>,
    surrounding_text: Option<(String, u32, u32)>,
    content_type: Option<(ContentHint, ContentPurpose)>,
    cursor_rectangle: Option<Rectangle<i32, Logical>>,
    text_change_cause: Option<ChangeCause>,
}

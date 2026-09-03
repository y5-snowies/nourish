use compositor_y5_window_interface_data::data::{WindowData, WindowFullscreen};
use std::cell::RefCell;
use smithay::desktop::{Space, Window};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::compositor::with_states;
use smithay::wayland::seat::WaylandFocus;
use uuid::Uuid;
use compositor_introspection_extraction_window_base::{InferredHints, MetaNode, ToplevelIcon, default_registry};
use compositor_introspection_inference_hint_base::ApplicationData;
use compositor_introspection_restoration_state_pending::pending::SessionKey;
use compositor_support_smithay_state_xdg_activation_dispatch::wire::ActivationDetails;
use compositor_support_smithay_state_window_ident::ident;
use smithay::wayland::xdg_activation::{XdgActivationToken, XdgActivationTokenData};

pub trait LoopWindow {
    fn window_data(&self) -> Option<&WindowData>;
    fn activations(&self) -> Vec<ActivationDetails>;

    /// The `xdg_session_management_v1` identity the client declared for this
    /// toplevel: present before the first commit, so it identifies the window
    /// rather than the launch. This is the CURRENT name — the one a placeholder should
    /// record, because it is what the client will restore under next run.
    fn session(&self) -> Option<SessionKey>;

    /// Every session key this window has declared this run, for MATCHING a placeholder.
    /// See the implementation: a client may retire the name it restored under
    /// before the window maps, in which case the placeholder is filed under the retired
    /// one and [`session`](Self::session) alone cannot find it.
    fn session_keys(&self) -> Vec<SessionKey>;

    fn uuid(&self) -> Option<Uuid>;

    /// The window's fullscreen restore data, if it is currently fullscreen.
    fn fullscreen(&self) -> Option<WindowFullscreen>;
    /// Whether the window is currently fullscreen.
    fn is_fullscreen(&self) -> bool;
    /// Set (or clear, with `None`) the window's fullscreen state.
    fn set_fullscreen(&self, value: Option<WindowFullscreen>);

    fn meta(&self, space: &Space<Window>, dh: &DisplayHandle) -> Option<MetaNode>;
    fn hints(&self, space: &Space<Window>, dh: &DisplayHandle) -> Option<InferredHints>;
    fn application(&self, space: &Space<Window>, dh: &DisplayHandle) -> Option<ApplicationData>;

    /// The `xdg_toplevel_icon_v1` icon this window's client declared, read live
    /// off its surface. This is the extra context `hints`/`application` feed to
    /// inference — it exists only where a window does, never on a `MetaNode`.
    fn toplevel_icon(&self) -> Option<ToplevelIcon>;
}

impl LoopWindow for Window {
    fn window_data(&self) -> Option<&WindowData> {
        self.user_data().get::<WindowData>()
    }

    fn uuid(&self) -> Option<Uuid> {
        self.window_data().map(|w| w.UUID)
    }

    fn fullscreen(&self) -> Option<WindowFullscreen> {
        self.user_data()
            .get::<RefCell<Option<WindowFullscreen>>>()
            .and_then(|cell| *cell.borrow())
    }

    fn is_fullscreen(&self) -> bool {
        self.fullscreen().is_some()
    }

    fn set_fullscreen(&self, value: Option<WindowFullscreen>) {
        self.user_data()
            .insert_if_missing(|| RefCell::<Option<WindowFullscreen>>::new(None));
        *self
            .user_data()
            .get::<RefCell<Option<WindowFullscreen>>>()
            .expect("fullscreen cell just inserted")
            .borrow_mut() = value;
    }

    fn meta(&self, space: &Space<Window>, dh: &DisplayHandle) -> Option<MetaNode> {
        compositor_introspection_extraction_window_base::extract_meta(self, space, dh)
    }

    fn hints(&self, space: &Space<Window>, dh: &DisplayHandle) -> Option<InferredHints> {
        let meta = self.meta(space, dh)?;
        let registry = default_registry();
        Some(compositor_introspection_extraction_window_base::extract_hints_with(
            &meta,
            &registry,
            self.toplevel_icon().as_ref(),
        ))
    }

    fn application(&self, space: &Space<Window>, dh: &DisplayHandle) -> Option<ApplicationData> {
        let meta = self.meta(space, dh)?;
        let registry = default_registry();

        // let meta_result = meta.clone();
        let hints = compositor_introspection_extraction_window_base::extract_hints_with(
            &meta,
            &registry,
            self.toplevel_icon().as_ref(),
        );
        Some(ApplicationData { meta, hints })
    }

    fn toplevel_icon(&self) -> Option<ToplevelIcon> {
        compositor_introspection_extraction_window_base::icon::toplevel::read(self)
    }

    fn session(&self) -> Option<SessionKey> {
        // X11's answer to the same question, from two properties rather than a
        // protocol and a server-side store. `WM_WINDOW_ROLE` is ICCCM's per-window
        // identity — a name the client picks and reuses for the SAME logical window on
        // every run, which is exactly what the name half means here.
        //
        // The id half is `WM_CLASS`, deliberately, and NOT `SM_CLIENT_ID`: an XSMP
        // client id is assigned per login session, so a key built on one would differ
        // after every reboot and pass 0 matches on exact equality of both fields. The
        // application identity is the durable thing X11 has.
        //
        // A window with no role yields NOTHING rather than a key with an empty name:
        // `(class, "")` is the same key for every window of an application, so the
        // first one to map would claim another's placeholder. A missing signal costs a
        // fallback to the token/pid pass; a wrong one restores the wrong window.
        if let Some((session_id, name)) = ident::x11_session_key(self) {
            return Some(SessionKey { session_id, name });
        }
        // `xdg_surface`, not `surface`: the wayland half is a role-bound protocol and
        // the X11 half is handled above.
        let surface = ident::xdg_surface(self)?;
        compositor_support_smithay_state_session_store::store::identity(&surface)
            .map(|i| SessionKey { session_id: i.session_id, name: i.name })
    }

    /// Every session key this window has declared: the name it RESTORED under
    /// first, then the name it currently holds. Both are needed to match a placeholder
    /// — a client may retire the restored name before the window maps (Chrome
    /// does, every run), and the placeholder is filed under whichever name that client
    /// last used. `session()` remains the CURRENT key, which is what a placeholder
    /// records so the chain rolls forward to the next run.
    fn session_keys(&self) -> Vec<SessionKey> {
        // X11 has one key and no history: `restored_from` exists because
        // `xdg_toplevel_session_v1.rename` lets a client re-key a toplevel mid-run,
        // and there is no such thing to re-key here — `WM_WINDOW_ROLE` is whatever the
        // window currently says it is.
        if self.is_x11() {
            return self.session().into_iter().collect();
        }
        let Some(surface) = ident::xdg_surface(self) else { return vec![] };
        let Some(identity) =
            compositor_support_smithay_state_session_store::store::identity(&surface)
        else {
            return vec![];
        };
        let mut keys = vec![];
        if let Some(restored) = identity.restored_from.filter(|r| *r != identity.name) {
            keys.push(SessionKey { session_id: identity.session_id.clone(), name: restored });
        }
        keys.push(SessionKey { session_id: identity.session_id, name: identity.name });
        keys
    }

    fn activations(&self) -> Vec<ActivationDetails> {
        // X11 declares the same thing through `_NET_STARTUP_ID` — startup notification
        // is the protocol xdg-activation replaced, and the token y5 hands a launch is
        // exported under BOTH names (`XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID`),
        // so an X11 client that sets the property is naming the very token we minted.
        //
        // Reading it here rather than leaving X11 to the `/proc` fallback matters: the
        // env route needs `_NET_WM_PID`, a readable `/proc/<pid>/environ` and the var
        // to have survived into it, while this is one property the X server already
        // tracked for us. The env route stays as the fallback for clients that consume
        // the variable without setting the property.
        if let Some(startup_id) = ident::x11_startup_id(self) {
            let token = XdgActivationToken::from(startup_id);
            // No `XdgActivationTokenData` was ever created for it — the client did not
            // go through `xdg_activation_v1` — so the correlation token is the whole
            // payload, which is all any caller reads.
            return vec![ActivationDetails { token, token_data: XdgActivationTokenData::default() }];
        }
        // CHECK: See WindowData re-insertion logic which explains how surface may outlive window and be created with new window.
        let Some(surface) = ident::xdg_surface(self) else { return vec![] };
        compositor_support_smithay_state_xdg_activation_dispatch::wire::activations(&surface)
    }
}

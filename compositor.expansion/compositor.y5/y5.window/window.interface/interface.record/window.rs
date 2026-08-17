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

pub trait LoopWindow {
    fn window_data(&self) -> Option<&WindowData>;
    fn activation(&self) -> Option<ActivationDetails>;

    /// The `xdg_session_management_v1` identity the client declared for this
    /// toplevel: present before the first commit and identical on every future
    /// run, so it identifies the window rather than the launch.
    fn session(&self) -> Option<SessionKey>;

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
        let surface = self.toplevel()?.wl_surface().clone();
        compositor_support_smithay_state_session_store::store::identity(&surface)
            .map(|i| SessionKey { session_id: i.session_id, name: i.name })
    }

    fn activation(&self) -> Option<ActivationDetails> {
        // CHECK: See WindowData re-insertion logic which explains how surface may outlive window and be created with new window.
        let surface = self.toplevel().unwrap_or_else(|| abort!("toplevel")).wl_surface();
        // It is possible to prefer surface / use it as fallback due to uncertain racing.
        with_states(&surface, |states| {
            let data = states.data_map.get::<ActivationDetails>().cloned();
            data
        })
        // self.user_data().get::<ActivationDetails>()
    }
}

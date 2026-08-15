use smithay::input::SeatHandler;
use smithay::reexports::wayland_protocols::wp::cursor_shape::v1::server::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1 as CursorShape;
use smithay::reexports::wayland_protocols::wp::fractional_scale::v1::server::{wp_fractional_scale_manager_v1 as frac_mgr, wp_fractional_scale_v1 as frac_v1};
use smithay::reexports::wayland_protocols::wp::linux_drm_syncobj::v1::server::wp_linux_drm_syncobj_manager_v1::WpLinuxDrmSyncobjManagerV1;
use smithay::reexports::wayland_protocols::wp::pointer_constraints::zv1::server::{zwp_confined_pointer_v1::ZwpConfinedPointerV1, zwp_locked_pointer_v1::ZwpLockedPointerV1, zwp_pointer_constraints_v1::ZwpPointerConstraintsV1};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::{wp_presentation, wp_presentation_feedback};
use smithay::reexports::wayland_protocols::wp::relative_pointer::zv1::server::{zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1, zwp_relative_pointer_v1::ZwpRelativePointerV1};
use smithay::reexports::wayland_protocols::wp::single_pixel_buffer::v1::server::wp_single_pixel_buffer_manager_v1::WpSinglePixelBufferManagerV1;
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::{zwp_text_input_manager_v3::ZwpTextInputManagerV3, zwp_text_input_v3::ZwpTextInputV3};
use smithay::reexports::wayland_protocols::wp::viewporter::server::{wp_viewport, wp_viewporter};
use smithay::reexports::wayland_protocols::xdg::activation::v1::server::xdg_activation_v1;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_decoration_manager_v1;
use smithay::reexports::wayland_protocols::xdg::foreign::zv2::server::{zxdg_exporter_v2::ZxdgExporterV2, zxdg_importer_v2::ZxdgImporterV2};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_wm_base::XdgWmBase;
use smithay::reexports::wayland_protocols::xdg::xdg_output::zv1::server::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use smithay::reexports::wayland_protocols_misc::zwp_input_method_v2::server::{zwp_input_method_manager_v2::ZwpInputMethodManagerV2, zwp_input_method_v2::ZwpInputMethodV2};
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_shell_v1::ZwlrLayerShellV1;
use smithay::reexports::wayland_server::protocol::{wl_compositor::WlCompositor, wl_data_device_manager::WlDataDeviceManager, wl_output::WlOutput, wl_seat::WlSeat, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_subcompositor::WlSubcompositor};
use smithay::reexports::wayland_server::{Dispatch as WLD, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::drm_syncobj::DrmSyncobjGlobalData;
use smithay::wayland::fractional_scale::{FractionalScaleData, FractionalScaleHandler};
use smithay::wayland::input_method::{InputMethodManagerGlobalData, InputMethodUserData};
use smithay::wayland::output::WlOutputData;
use smithay::wayland::pointer_constraints::PointerConstraintUserData;
use smithay::wayland::presentation::PresentationData;
use smithay::wayland::relative_pointer::RelativePointerUserData;
use smithay::wayland::seat::{SeatGlobalData, WaylandFocus};
use smithay::wayland::selection::data_device::DataDeviceHandler;
use smithay::wayland::shell::wlr_layer::WlrLayerShellGlobalData;
use smithay::wayland::shell::xdg::decoration::XdgDecorationManagerGlobalData;
use smithay::wayland::shm::{ShmHandler, ShmPoolUserData};
use smithay::wayland::text_input::TextInputUserData;
use smithay::wayland::viewporter::ViewportState;
use smithay::wayland::xdg_activation::XdgActivationHandler;
use smithay::wayland::xdg_foreign::XdgForeignHandler;
use compositor_support_smithay_dispatch_state_base::state::DispatchWire;

pub trait FactoryBounds: DispatchWire + SeatHandler
    + GlobalDispatch<WlDataDeviceManager, GlobalData> + DataDeviceHandler
    + GlobalDispatch<XdgWmBase, GlobalData> + GlobalDispatch<WlSeat, SeatGlobalData<Self>>
    + GlobalDispatch<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, XdgDecorationManagerGlobalData>
    + WLD<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, GlobalData>
    + GlobalDispatch<WlShm, GlobalData> + WLD<WlShm, GlobalData> + WLD<WlShmPool, ShmPoolUserData>
    + BufferHandler + ShmHandler
    + GlobalDispatch<WlOutput, WlOutputData> + GlobalDispatch<ZxdgOutputManagerV1, GlobalData>
    + GlobalDispatch<ZwlrLayerShellV1, WlrLayerShellGlobalData>
    + GlobalDispatch<WlCompositor, GlobalData> + GlobalDispatch<WlSubcompositor, GlobalData>
    + GlobalDispatch<wp_presentation::WpPresentation, PresentationData>
    + WLD<wp_presentation::WpPresentation, PresentationData>
    + WLD<wp_presentation_feedback::WpPresentationFeedback, GlobalData>
    + GlobalDispatch<xdg_activation_v1::XdgActivationV1, GlobalData>
    + WLD<xdg_activation_v1::XdgActivationV1, GlobalData> + XdgActivationHandler
    + GlobalDispatch<wp_viewporter::WpViewporter, GlobalData>
    + WLD<wp_viewporter::WpViewporter, GlobalData> + WLD<wp_viewport::WpViewport, ViewportState>
    + GlobalDispatch<frac_mgr::WpFractionalScaleManagerV1, GlobalData>
    + WLD<frac_mgr::WpFractionalScaleManagerV1, GlobalData>
    + WLD<frac_v1::WpFractionalScaleV1, FractionalScaleData> + FractionalScaleHandler
    + XdgForeignHandler + GlobalDispatch<ZxdgExporterV2, GlobalData> + GlobalDispatch<ZxdgImporterV2, GlobalData>
    + GlobalDispatch<ZwpTextInputManagerV3, GlobalData> + WLD<ZwpTextInputManagerV3, GlobalData>
    + WLD<ZwpTextInputV3, TextInputUserData>
    + GlobalDispatch<ZwpInputMethodManagerV2, InputMethodManagerGlobalData>
    + WLD<ZwpInputMethodManagerV2, GlobalData> + WLD<ZwpInputMethodV2, InputMethodUserData<Self>>
    + GlobalDispatch<CursorShape, GlobalData> + WLD<CursorShape, GlobalData>
    + GlobalDispatch<WpLinuxDrmSyncobjManagerV1, DrmSyncobjGlobalData>
    + GlobalDispatch<ZwpRelativePointerManagerV1, GlobalData>
    + WLD<ZwpRelativePointerManagerV1, GlobalData>
    + WLD<ZwpRelativePointerV1, RelativePointerUserData<Self>>
    + GlobalDispatch<ZwpPointerConstraintsV1, GlobalData>
    + WLD<ZwpPointerConstraintsV1, GlobalData>
    + WLD<ZwpConfinedPointerV1, PointerConstraintUserData<Self>>
    + WLD<ZwpLockedPointerV1, PointerConstraintUserData<Self>>
    + GlobalDispatch<WpSinglePixelBufferManagerV1, GlobalData>
    + WLD<WpSinglePixelBufferManagerV1, GlobalData> + 'static
where
    <Self as SeatHandler>::PointerFocus: WaylandFocus,
    <Self as SeatHandler>::KeyboardFocus: WaylandFocus,
{}

// `impl FactoryBounds for Dispatch` lives here (FactoryBounds is local to this
// crate; `Dispatch` comes from state.base which this crate depends on). All the
// required GlobalDispatch/Dispatch supertraits are provable because state.base's
// `delegate_dispatch2!(Dispatch)` + handler impls are in scope.
// document/SMITHAY_DECOUPLING.md P2 flip.
impl FactoryBounds for compositor_support_smithay_dispatch_state_base::state::Dispatch {}

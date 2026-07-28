use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use smithay::reexports::wayland_protocols::wp::color_management::v1::server::{
    wp_color_manager_v1::{Feature, Primaries, RenderIntent, TransferFunction, WpColorManagerV1},
    wp_image_description_v1::WpImageDescriptionV1,
};
use smithay::reexports::wayland_server::{DisplayHandle, GlobalDispatch, Resource};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::with_states;

pub static IDENTITY: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Clone, Copy)]
pub struct SurfaceColor {
    pub transfer: Option<TransferFunction>,
    pub primaries: Option<Primaries>,
}
impl SurfaceColor {
    pub fn is_hdr(&self) -> bool {
        matches!(self.transfer, Some(TransferFunction::St2084Pq) | Some(TransferFunction::Hlg))
    }
}

#[derive(Default)]
pub struct ParamsState { pub transfer: Option<TransferFunction>, pub primaries: Option<Primaries> }

#[derive(Debug, Clone, Copy, Default)]
pub struct ImageDescData { pub transfer: Option<TransferFunction>, pub primaries: Option<Primaries> }

pub fn transfer_code(tf: Option<TransferFunction>) -> u8 {
    match tf {
        Some(TransferFunction::St2084Pq) => 1,
        Some(TransferFunction::Hlg) => 2,
        Some(TransferFunction::ExtLinear) => 3,
        _ => 0,
    }
}

pub fn store_surface_color(surface: &WlSurface, color: Option<SurfaceColor>) {
    let hdr = color.map(|c| compositor_kernel_graphic_color_surface_base::SurfaceHdr {
        transfer: transfer_code(c.transfer),
        is_hdr: c.is_hdr(),
    });
    with_states(surface, |states| { compositor_kernel_graphic_color_surface_base::set(states, hdr); });
}

pub fn send_ready(desc: &WpImageDescriptionV1) {
    let id = IDENTITY.fetch_add(1, Ordering::Relaxed);
    if desc.version() >= 2 { desc.ready2(0, id); } else { desc.ready(id); }
}

pub fn wenum<T: TryFrom<u32>>(w: smithay::reexports::wayland_server::WEnum<T>) -> Option<T> {
    w.into_result().ok()
}

pub fn create_global<W: GlobalDispatch<WpColorManagerV1, ()> + 'static>(dh: &DisplayHandle) {
    if !compositor_model_environment_config_base::base::get().hdr { return; }
    dh.create_global::<W, WpColorManagerV1, ()>(1, ());
}

pub fn bind_color_manager(mgr: WpColorManagerV1) {
    mgr.supported_intent(RenderIntent::Perceptual);
    mgr.supported_feature(Feature::Parametric);
    mgr.supported_feature(Feature::SetLuminances);
    mgr.supported_feature(Feature::SetMasteringDisplayPrimaries);
    for tf in [TransferFunction::Srgb, TransferFunction::Gamma22, TransferFunction::ExtLinear,
               TransferFunction::St2084Pq, TransferFunction::Hlg] { mgr.supported_tf_named(tf); }
    for p in [Primaries::Srgb, Primaries::DisplayP3, Primaries::Bt2020] { mgr.supported_primaries_named(p); }
    mgr.done();
}


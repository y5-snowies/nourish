use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::{ImportAll, ImportMem};
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use compositor_orchestration_seat_pointer_element::element::PointerRenderElement;
use compositor_monitor_compositor_iced_base::IcedRenderElement;

pub use compositor_orchestration_draw_scene_preimported::preimported::PreImported;

smithay::render_elements! {
    pub SceneElement<R> where R: ImportAll + ImportMem + SceneDispatch;
    Canvas = compositor_y5_canvas_draw_element::element::Element<R>,
    Layershell = WaylandSurfaceRenderElement<R>,
    Surface = IcedRenderElement,
    /// World-space iced surface clipped to a viewport pane (GLES path).
    SurfaceCropped = smithay::backend::renderer::element::utils::CropRenderElement<IcedRenderElement>,
    /// World-space iced surface (dmabuf-imported) clipped to a pane (native path).
    TextureCropped = smithay::backend::renderer::element::utils::CropRenderElement<PreImported<R>>,
    Pointer = PointerRenderElement<R>,
    Background2D = compositor_background_two_draw_element::element::ParallaxBackground,
    /// Parallax background hard-clipped to a viewport pane (floating panes — the
    /// shader's clear would otherwise paint beyond the pane's rect).
    Background2DCropped = smithay::backend::renderer::element::utils::CropRenderElement<compositor_background_two_draw_element::element::ParallaxBackground>,
    Background3D = compositor_support_bevy_core_compositor_base::BevyRenderElement,
    Texture = PreImported<R>,
    Sentinel = SolidColorRenderElement,
}

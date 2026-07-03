//! `DrawNode` — the owned, renderer-agnostic draw currency. Scene contributors
//! (and, increasingly, systems' `draw()`) describe WHAT to draw and at WHICH
//! `Layer`; the single `lower()` seam turns a node into the renderer's
//! `SceneElement` at the backend boundary (importing dmabuf into a native
//! texture on renderers that prefer it, passthrough on GLES). This replaces the
//! old implicit push-order assembly: layering is now explicit and the
//! node→element lowering lives in exactly one place.

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::utils::{Physical, Point, Scale};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::backend::renderer::{ImportAll, ImportDma, ImportMem, Renderer, Texture};
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use compositor_orchestration_draw_scene_element::element::{PreImported, SceneElement};
use compositor_support_system_world_frame_base::base::Layer;

type Iced = compositor_monitor_compositor_iced_base::IcedRenderElement;
type Bevy = compositor_support_bevy_core_compositor_base::BevyRenderElement;

/// One unit of drawable content, generic over the active renderer `R`.
/// A renderer-agnostic wl_surface tree to draw: systems/contributors carry the
/// surface + placement, and the backend builds the `WaylandSurfaceRenderElement`s
/// at `lower()` time (one tree → many elements for subsurfaces). This is the
/// goal-(B) shape — no `<R>`, no smithay render element constructed by the
/// contributor.
pub struct SurfaceNode {
    pub surface: WlSurface,
    pub location: Point<i32, Physical>,
    pub scale: f64,
    pub alpha: f32,
}

pub enum DrawNode<R: Renderer> {
    /// Renderer-agnostic surface tree (lowered to layershell elements).
    Surface(SurfaceNode),
    Pointer(compositor_orchestration_seat_pointer_element::element::PointerRenderElement<R>),
    Layershell(WaylandSurfaceRenderElement<R>),
    Canvas(compositor_y5_canvas_draw_element::element::Element<R>),
    /// iced UI surface (world or screen); imported via dmabuf on native renderers.
    Iced(Iced),
    /// World iced surface clipped to a viewport pane's physical rect.
    IcedCropped {
        elem: Iced,
        crop: smithay::utils::Rectangle<i32, Physical>,
    },
    /// bevy 3D background; imported via dmabuf on native renderers.
    Background3D(Bevy),
    Background2D(compositor_background_two_draw_element::element::ParallaxBackground),
    /// Parallax background clipped to a viewport pane rect (floating panes).
    Background2DCropped(
        smithay::backend::renderer::element::utils::CropRenderElement<
            compositor_background_two_draw_element::element::ParallaxBackground,
        >,
    ),
    /// A texture already imported into `R`.
    Texture(PreImported<R>),
    Solid(SolidColorRenderElement),
}

/// A layered collection of draw nodes. Contributors push at explicit `Layer`
/// bands (BACKGROUND..POINTER); `lower()` orders them topmost-first and turns
/// them into the renderer's `SceneElement` list.
pub struct Plan<R: Renderer> {
    nodes: Vec<(Layer, DrawNode<R>)>,
}

impl<R: Renderer> Default for Plan<R> {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

impl<R> Plan<R>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, layer: Layer, node: DrawNode<R>) {
        self.nodes.push((layer, node));
    }

    pub fn extend<I: IntoIterator<Item = DrawNode<R>>>(&mut self, layer: Layer, nodes: I) {
        for node in nodes {
            self.nodes.push((layer, node));
        }
    }

    /// Order topmost-first (higher Layer drawn on top → emitted first, matching
    /// smithay's first-is-front element order) and lower each node. Nodes whose
    /// dmabuf import fails are dropped for this frame.
    pub fn lower(mut self, renderer: &mut R) -> Vec<SceneElement<R>> {
        self.nodes.sort_by(|a, b| b.0.cmp(&a.0));
        self.nodes
            .into_iter()
            .flat_map(|(_, node)| node.lower(renderer))
            .collect()
    }
}

impl<R> DrawNode<R>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    pub fn lower(self, renderer: &mut R) -> Vec<SceneElement<R>> {
        match self {
            DrawNode::Surface(n) => render_elements_from_surface_tree::<R, WaylandSurfaceRenderElement<R>>(
                renderer,
                &n.surface,
                n.location,
                Scale::from(n.scale),
                n.alpha,
                Kind::Unspecified,
            )
            .into_iter()
            .map(SceneElement::Layershell)
            .collect(),
            DrawNode::Pointer(e) => vec![SceneElement::Pointer(e)],
            DrawNode::Layershell(e) => vec![SceneElement::Layershell(e)],
            DrawNode::Canvas(e) => vec![SceneElement::Canvas(e)],
            DrawNode::Background2D(e) => vec![SceneElement::Background2D(e)],
            DrawNode::Background2DCropped(e) => vec![SceneElement::Background2DCropped(e)],
            DrawNode::Texture(e) => vec![SceneElement::Texture(e)],
            DrawNode::Solid(e) => vec![SceneElement::Sentinel(e)],
            DrawNode::Iced(e) => {
                if !R::prefers_dmabuf() {
                    return vec![SceneElement::Surface(e)];
                }
                import_texture(renderer, &e.dmabuf, e.location, e.size, e.world_zoom, e.id, e.commit_counter).into_iter().collect()
            }
            DrawNode::IcedCropped { elem, crop } => {
                use smithay::backend::renderer::element::utils::CropRenderElement;
                // Geometry is physical and scale-independent for both element types,
                // so the crop scale is irrelevant.
                if !R::prefers_dmabuf() {
                    return CropRenderElement::from_element(elem, Scale::from(1.0), crop)
                        .map(SceneElement::SurfaceCropped)
                        .into_iter()
                        .collect();
                }
                match renderer.import_dmabuf(&elem.dmabuf, None) {
                    Ok(texture) => {
                        let pre = PreImported {
                            texture,
                            location: elem.location,
                            size: elem.size,
                            world_zoom: elem.world_zoom,
                            id: elem.id,
                            commit: elem.commit_counter,
                        };
                        CropRenderElement::from_element(pre, Scale::from(1.0), crop)
                            .map(SceneElement::TextureCropped)
                            .into_iter()
                            .collect()
                    }
                    Err(err) => {
                        error!("draw.node: dmabuf import (cropped iced) failed: {err}");
                        vec![]
                    }
                }
            }
            DrawNode::Background3D(e) => {
                if !R::prefers_dmabuf() {
                    return vec![SceneElement::Background3D(e)];
                }
                import_texture(renderer, &e.dmabuf, e.location, e.size, e.world_zoom, e.id, e.commit_counter).into_iter().collect()
            }
        }
    }
}

/// Import a dmabuf into a native `PreImported` texture (drops the node on failure).
#[allow(clippy::too_many_arguments)]
fn import_texture<R>(
    renderer: &mut R,
    dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
    location: smithay::utils::Point<i32, smithay::utils::Physical>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    world_zoom: f64,
    id: smithay::backend::renderer::element::Id,
    commit: smithay::backend::renderer::utils::CommitCounter,
) -> Option<SceneElement<R>>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    match renderer.import_dmabuf(dmabuf, None) {
        Ok(texture) => Some(SceneElement::Texture(PreImported { texture, location, size, world_zoom, id, commit })),
        Err(err) => {
            error!("draw.node: dmabuf import into the active renderer failed: {err}");
            None
        }
    }
}

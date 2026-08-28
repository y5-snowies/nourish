//! `DrawNode` — the owned, renderer-agnostic draw currency. Scene contributors
//! (and, increasingly, systems' `draw()`) describe WHAT to draw and at WHICH
//! `Layer`; the single `lower()` seam turns a node into the renderer's
//! `SceneElement` at the backend boundary (importing dmabuf into a native
//! texture on renderers that prefer it, passthrough on GLES). This replaces the
//! old implicit push-order assembly: layering is now explicit and the
//! node→element lowering lives in exactly one place.

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement};
use smithay::backend::renderer::element::{Element, Kind};
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
    /// Canvas content: a window's surfaces and decorations, the select box, the
    /// cursor solid. `flags` are the `window.descriptor` bits of the WINDOW this
    /// element belongs to, or zero for the canvas furniture that belongs to none —
    /// the one thing about a canvas node that cannot be recovered from its variant
    /// at `lower()` time. See `ElementMeta::flags`.
    Canvas {
        elem: compositor_y5_canvas_draw_element::element::Element<R>,
        flags: u32,
        times: [f32; 12],
    },
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
    /// Carried unwrapped (like `IcedCropped`) so the off-thread path can swap the
    /// shader element for the worker's texture before the crop is applied.
    Background2DCropped {
        elem: compositor_background_two_draw_element::element::ParallaxBackground,
        crop: smithay::utils::Rectangle<i32, Physical>,
    },
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
    /// dmabuf import fails are dropped for this frame. Returns the elements plus a
    /// lockstep [`ElementMeta`] per element: its space (`World` for client
    /// windows + iced-world panels, so the renderer can restrict AA to world
    /// content) and whether it's a client `window` (only those feed the shader
    /// pipeline's window-rects/window-textures set).
    pub fn lower(
        mut self,
        renderer: &mut R,
    ) -> (Vec<SceneElement<R>>, Vec<compositor_orchestration_draw_dispatch_frame::ElementMeta>) {
        use compositor_orchestration_draw_dispatch_frame::ElementMeta;
        self.nodes.sort_by(|a, b| b.0.cmp(&a.0));
        let mut elements = Vec::new();
        let mut meta = Vec::new();
        for (_, node) in self.nodes {
            // World content is exactly windows + iced-world panels; everything
            // else (bevy, parallax, screen iced, layershell, pointer, solids) is
            // screen-space. Client windows are tagged WINDOW (a subset of world):
            // only they feed the shader pipeline's window-rects/window-textures
            // set. Iced-world panels/placeholders stay WORLD — AA-eligible, but
            // kept out of the window-set so glass/window-glow never treat a
            // placeholder as a window.
            let m = match &node {
                DrawNode::Canvas { flags, times, .. } => ElementMeta::window(*flags, *times),
                DrawNode::IcedCropped { .. } => ElementMeta::WORLD,
                DrawNode::Background2D(_) | DrawNode::Background2DCropped { .. } => {
                    ElementMeta::BACKGROUND
                }
                _ => ElementMeta::SCREEN,
            };
            for e in node.lower(renderer) {
                elements.push(e);
                meta.push(m);
            }
        }
        (elements, meta)
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
            DrawNode::Canvas { elem, .. } => vec![SceneElement::Canvas(elem)],
            // Off-thread background: the worker already rendered this pane, so
            // sample its dmabuf instead of running the shader inside the
            // compositor's own command buffer. `worker_frame` is `None` both when
            // no worker is configured (inline path below) and before this pane's
            // first frame lands — in the latter case we draw nothing at all
            // rather than sample a buffer that was never written.
            DrawNode::Background2D(e) => {
                // Publish the band's multipass machinery OUT-OF-BAND, before any
                // path below can lose the element: smithay's occlusion culls may
                // eat the band (a window covering the output does), and the
                // bundle's after-content passes must not go with it. See
                // `SceneDispatch::set_band_machinery`.
                if R::prefers_dmabuf() {
                    renderer.set_band_machinery(e.machinery_pass());
                }
                // `worker_can_render` keeps a multipass bundle off the worker,
                // which renders a single pass and would silently draw the wrong
                // background for it. See `ParallaxBackground::worker_can_render`.
                if R::prefers_dmabuf() && e.offthread && e.worker_can_render() {
                    let Some((dmabuf, generation)) = e.worker_frame() else { return vec![] };
                    // Commit from the worker's publish generation, NOT the
                    // element's own per-frame counter: unchanged generation ⇒ no
                    // damage ⇒ smithay skips the region instead of re-reading and
                    // re-blending an identical full-screen buffer every frame.
                    return import_texture(
                        renderer, &dmabuf, e.location(Scale::from(1.0)),
                        e.geometry(Scale::from(1.0)).size, 1.0, e.id().clone(), generation.into(),
                    )
                    .into_iter()
                    .collect();
                }
                // Stage 4: the worker decorates rather than replaces, so the
                // element still draws its head band here — but the worker has to
                // keep being pumped, and its decorated band published, or the
                // compositor has nothing to present.
                if R::prefers_dmabuf() && e.offloads_after_band() {
                    renderer.set_after_band(e.after_band());
                }
                vec![SceneElement::Background2D(e)]
            }
            DrawNode::Background2DCropped { elem, crop } => {
                use smithay::backend::renderer::element::utils::CropRenderElement;
                // Same out-of-band publish as the uncropped arm above.
                if R::prefers_dmabuf() {
                    renderer.set_band_machinery(elem.machinery_pass());
                }
                if R::prefers_dmabuf() && elem.offthread && elem.worker_can_render() {
                    let Some((dmabuf, generation)) = elem.worker_frame() else { return vec![] };
                    let Some(pre) = import_pre(
                        renderer, &dmabuf, elem.location(Scale::from(1.0)),
                        elem.geometry(Scale::from(1.0)).size, 1.0,
                        elem.id().clone(), generation.into(),
                    ) else { return vec![] };
                    return CropRenderElement::from_element(pre, Scale::from(1.0), crop)
                        .map(SceneElement::TextureCropped)
                        .into_iter()
                        .collect();
                }
                CropRenderElement::from_element(elem, Scale::from(1.0), crop)
                    .map(SceneElement::Background2DCropped)
                    .into_iter()
                    .collect()
            }
            DrawNode::Texture(e) => vec![SceneElement::Texture(e)],
            DrawNode::Solid(e) => vec![SceneElement::Sentinel(e)],
            DrawNode::Iced(e) => {
                // Same rule as `Background3D` below. A GLES pass samples the
                // element's own `GlesTexture`, and only an INLINE-rasterized surface
                // has one: with the off-thread iced worker the element carries
                // `texture: None`, which makes `IcedRenderElement::draw` a silent
                // no-op — Ok, and nothing painted. So import the dmabuf instead.
                //
                // Gating on `prefers_dmabuf` ALONE missed that, because it asks
                // about the renderer rather than about this element. Winit's picker
                // and lock passes are GLES even when the scene pass is Vulkan, so
                // those overlays drew nothing at all there while udev was fine.
                if !R::prefers_dmabuf() && e.texture.is_some() {
                    return vec![SceneElement::Surface(e)];
                }
                import_texture(renderer, &e.dmabuf, e.location, e.size, e.world_zoom, e.id, e.commit_counter).into_iter().collect()
            }
            DrawNode::IcedCropped { elem, crop } => {
                use smithay::backend::renderer::element::utils::CropRenderElement;
                // Geometry is physical and scale-independent for both element types,
                // so the crop scale is irrelevant. `texture.is_some()` for the same
                // reason as the arm above.
                if !R::prefers_dmabuf() && elem.texture.is_some() {
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
                // A GLES pass samples the instance's own `GlesTexture` — but only
                // an INLINE instance has one. Off-thread (ring-worker) slots carry
                // no GLES view: building one needs `&mut GlesRenderer`, the one
                // thing that cannot leave the compositor thread. `texture: None`
                // makes `BevyRenderElement::draw` a no-op, so a GLES pass MUST
                // import the dmabuf here or the scene silently draws nothing.
                //
                // Not hypothetical: winit composes its picker and lock passes
                // through GLES even when the scene pass is on Vulkan, so with
                // triple buffering on, the world picker opened to an empty screen.
                if !R::prefers_dmabuf() && e.texture.is_some() {
                    return vec![SceneElement::Background3D(e)];
                }
                import_texture(renderer, &e.dmabuf, e.location, e.size, e.world_zoom, e.id, e.commit_counter).into_iter().collect()
            }
        }
    }
}

/// Import a dmabuf into a `PreImported` (drops the node on failure).
#[allow(clippy::too_many_arguments)]
fn import_pre<R>(
    renderer: &mut R,
    dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf,
    location: smithay::utils::Point<i32, smithay::utils::Physical>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    world_zoom: f64,
    id: smithay::backend::renderer::element::Id,
    commit: smithay::backend::renderer::utils::CommitCounter,
) -> Option<PreImported<R>>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    match renderer.import_dmabuf(dmabuf, None) {
        Ok(texture) => Some(PreImported { texture, location, size, world_zoom, id, commit }),
        Err(err) => {
            error!("draw.node: dmabuf import into the active renderer failed: {err}");
            None
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

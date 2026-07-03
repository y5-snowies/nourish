//! `IcedRenderElement`: the `RenderElement<GlesRenderer>` you add to your
//! render list.
//!
//! Built per-frame from an `IcedItem` (via `IcedItem::element_in`).
//! Cheap to construct: clones the `GlesTexture` (Arc-like internally) and
//! copies placement metadata.
//!
//! ## Geometry & zoom
//! For Screen-space items, `world_zoom == 1.0` and the element renders
//! at its natural texture size. For World-space items, `world_zoom`
//! matches the active `Transform::zoom`; the element's `geometry()`
//! returns the texture size multiplied by zoom so smithay's renderer
//! stretches the destination quad. The location passed in is already
//! the on-screen physical pixel position.
//!
//! ## Damage policy (v1)
//! Full-rect damage when the commit counter advances. Iced doesn't
//! expose sub-rect damage publicly. For "dozens of small UIs" this is
//! fine. When the camera transform changes, the registry bumps the
//! commit counter on every world-space item so smithay damages their
//! old and new screen rects correctly.

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::gles::GlesTexture;
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::RendererSuper;
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform as SmithayTransform};
use crate::IcedSpace;

#[derive(Clone)]
pub struct IcedRenderElement {
    pub texture: GlesTexture,
    /// The surface's underlying dmabuf (strict accessor), for renderers that
    /// import the iced output natively instead of sampling `texture`.
    pub dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
    pub space: IcedSpace,
    /// On-screen physical pixel position (camera-transformed for World items).
    pub location: Point<i32, Physical>,
    /// Natural texture size in physical pixels.
    pub size: Size<i32, Physical>,
    /// Zoom factor applied to the destination rect. 1.0 for Screen items;
    /// matches camera zoom for World items.
    pub world_zoom: f64,
    pub id: Id,
    pub commit_counter: CommitCounter,
    /// Owning physical output tag, if the source surface is bound to one (see
    /// `IcedItem::output`). The scene builder draws a bound element only on its
    /// output; `None` follows the default single-output placement.
    pub output: Option<String>,
}

impl std::fmt::Debug for IcedRenderElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IcedRenderElement")
            .field("location", &self.location)
            .field("size", &self.size)
            .field("world_zoom", &self.world_zoom)
            .field("commit", &self.commit_counter)
            .finish()
    }
}

impl IcedRenderElement {
    /// Returns the on-screen destination size (size scaled by zoom).
    fn dest_size(&self) -> Size<i32, Physical> {
        if (self.world_zoom - 1.0).abs() < f64::EPSILON {
            self.size
        } else {
            Size::from((
                (self.size.w as f64 * self.world_zoom) as i32,
                (self.size.h as f64 * self.world_zoom) as i32,
            ))
        }
    }
}

impl Element for IcedRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit_counter
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        // Sample the entire texture (natural size).
        Rectangle::from_loc_and_size((0.0, 0.0), (self.size.w as f64, self.size.h as f64))
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::from_loc_and_size(self.location, self.dest_size())
    }

    fn location(&self, _scale: Scale<f64>) -> Point<i32, Physical> {
        self.location
    }

    fn transform(&self) -> SmithayTransform {
        SmithayTransform::Normal
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit != Some(self.commit_counter) {
            vec![self.geometry(scale)].into_iter().collect()
        } else {
            DamageSet::default()
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        1.0
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

// Renderer-agnostic: the iced-rendered GLES texture is drawn through the
// `SceneDispatchFrame` seam — real on GlesFrame, a no-op on VulkanFrame until
// the iced output is exposed as a renderer-native (dmabuf-imported) texture.
impl<R: SceneDispatch> RenderElement<R> for IcedRenderElement {
    fn draw(
        &self,
        frame: &mut <R as RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), <R as RendererSuper>::Error> {
        R::draw_prerendered_texture(frame, &self.texture, src, dst, damage, 1.0)
    }
}

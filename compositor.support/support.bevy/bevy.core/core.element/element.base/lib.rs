//! `BevyRenderElement`: the render element you add to your render list.
//! The Bevy app has already rendered into a dmabuf-backed wgpu texture; we
//! sample the corresponding GLES texture and composite it at screen coords.

use compositor_support_bevy_core_space_base::BevySpace;
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use smithay::backend::renderer::RendererSuper;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::gles::GlesTexture;
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{
    Buffer, Physical, Point, Rectangle, Scale, Size, Transform as SmithayTransform,
};

#[derive(Clone)]
pub struct BevyRenderElement {
    /// `None` on the off-thread path, which is Vulkan-only: `node.rs`'s
    /// `Background3D` arm imports `dmabuf` natively there and never reads this.
    pub texture: Option<GlesTexture>,
    /// The surface's underlying dmabuf (strict accessor) for native (Vulkan)
    /// import; GLES samples `texture`.
    pub dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
    pub space: BevySpace,
    /// On-screen physical pixel position (camera-transformed for World items).
    pub location: Point<i32, Physical>,
    /// Natural texture size in physical pixels.
    pub size: Size<i32, Physical>,
    /// Zoom factor applied to the destination rect. 1.0 for Screen items;
    /// matches camera zoom for World items.
    pub world_zoom: f64,
    pub id: Id,
    pub commit_counter: CommitCounter,
}

impl std::fmt::Debug for BevyRenderElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BevyRenderElement")
            .field("location", &self.location)
            .field("size", &self.size)
            .field("world_zoom", &self.world_zoom)
            .field("commit", &self.commit_counter)
            .finish()
    }
}

impl BevyRenderElement {
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

impl Element for BevyRenderElement {
    fn id(&self) -> &Id { &self.id }
    fn current_commit(&self) -> CommitCounter { self.commit_counter }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_loc_and_size((0.0, 0.0), (self.size.w as f64, self.size.h as f64))
    }
    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::from_loc_and_size(self.location, self.dest_size())
    }
    fn location(&self, _scale: Scale<f64>) -> Point<i32, Physical> { self.location }
    fn transform(&self) -> SmithayTransform { SmithayTransform::Normal }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        if commit != Some(self.commit_counter) {
            vec![self.geometry(scale)].into_iter().collect()
        } else {
            DamageSet::default()
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> { OpaqueRegions::default() }
    fn alpha(&self) -> f32 { 1.0 }
    fn kind(&self) -> Kind { Kind::Unspecified }
}

// Renderer-agnostic: the bevy-rendered GLES texture is drawn through the
// `SceneDispatchFrame` seam — real on GlesFrame, a no-op on VulkanFrame until
// the bevy output is exposed as a renderer-native (dmabuf-imported) texture.
impl<R: SceneDispatch> RenderElement<R> for BevyRenderElement {
    fn draw(
        &self,
        frame: &mut <R as RendererSuper>::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), <R as RendererSuper>::Error> {
        // No GLES view means the off-thread (Vulkan) path, where the compositor
        // composites the imported dmabuf instead and this element is never asked
        // to draw itself. Nothing to do rather than an error.
        match &self.texture {
            Some(t) => R::draw_prerendered_texture(frame, t, src, dst, damage, 1.0),
            None => Ok(()),
        }
    }
}

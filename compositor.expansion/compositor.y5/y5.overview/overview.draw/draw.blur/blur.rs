//! Gaussian-ish blur of the frozen desktop, baked into a fresh full-res dmabuf.
//!
//! Chained LINEAR downsample (full → ½ → ¼ → ⅛) — each exact-halving blit is a
//! proper 2×2 box average, so the chain is a real low-pass (no aliasing) — then a
//! single LINEAR upscale back to full. All filtering is explicit `Linear`, so the
//! result is smooth regardless of the draw-time sampler. Cross-renderer safe: the
//! output is a dmabuf the scene imports + draws 1:1. Returns `None` on any
//! allocation/blit failure (the caller falls back to the sharp snapshot).

use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{Bind, Blit, ImportDma, TextureFilter};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::utils::{Physical, Point, Rectangle, Size};
use compositor_support_bevy_core_alloc_base::{AllocatedDmabuf, allocate_dmabuf_negotiated};
use compositor_y5_graphic_capture_registry::SnapshotHandle;

/// The blur output is sampled by the COMPOSITING renderer, so it must carry a
/// modifier that renderer accepts. These were allocated with no modifier list at all,
/// which on the Vulkan path corrupted UNRELATED windows: the driver chose freely and
/// picked an NVIDIA block-linear kind outside the six it reports as importable
/// (`0x0300000000E08014`, not the `0x…60601{0..5}` family). GLES imports anything via
/// EGLImage so the blits still succeeded, but the compositor's `vkCreateImage`
/// returned `VK_ERROR_FORMAT_NOT_SUPPORTED`, the view had "no supported format
/// features", and the draw proceeded with it bound — a garbage descriptor sampled in
/// the same command buffer as every other window.
///
/// The layer asks the right question for this consumer: what the active compositing
/// renderer can import ∩ what GLES (which writes these) can. Empty ⇒
/// return `None` and keep the sharp snapshot. Never fall back to the implicit path —
/// that IS the bug above, and `allocate_dmabuf_negotiated` silently takes it when
/// handed an empty list.
fn modifiers(formats: &compositor_kernel_graphic_format_registrar_base::registrar::Registrar) -> Option<(Fourcc, Vec<Modifier>)> {
    let (fourcc, mods) = compositor_kernel_graphic_format_answer_base::answer::producer_formats(formats, compositor_kernel_graphic_format_answer_base::answer::Consumer::OverviewBlur);
    if mods.is_empty() {
        warn!(
            "overview blur: no modifier the compositor can import and GLES can write for \
             {fourcc:?}; skipping the blur rather than allocating implicitly"
        );
        return None;
    }
    Some((fourcc, mods))
}

fn rect(w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new(Point::from((0, 0)), Size::from((w.max(1), h.max(1))))
}

fn blit(gles: &mut GlesRenderer, from: &mut Dmabuf, to: &mut Dmabuf, src: Rectangle<i32, Physical>, dst: Rectangle<i32, Physical>) -> Option<()> {
    let s = gles.bind(from).ok()?;
    let mut t = gles.bind(to).ok()?;
    let sync = gles.blit(&s, &mut t, src, dst, TextureFilter::Linear).ok()?;
    // Block until the GPU has consumed this stage — the intermediate dmabufs are
    // dropped when `blur` returns, so each must be done being read first.
    let _ = sync.wait();
    Some(())
}

/// Produce a full-res blurred copy of `snap`'s desktop frame.
pub fn blur(
    formats: &compositor_kernel_graphic_format_registrar_base::registrar::Registrar,
    gles: &mut GlesRenderer,
    node: &str,
    snap: &SnapshotHandle,
) -> Option<AllocatedDmabuf> {
    let full = snap.size();
    let (w, h) = (full.w.max(1), full.h.max(1));
    // Negotiated for all four, not just `out`: the intermediates are bound as GLES
    // render targets here, but they are the same allocation path and an implicit
    // modifier in any of them is the bug described on `modifiers` above.
    let (fourcc, mods) = modifiers(formats)?;
    let alloc = |dw: i32, dh: i32| {
        allocate_dmabuf_negotiated(node, dw.max(1) as u32, dh.max(1) as u32, fourcc, &mods).ok()
    };
    let half = alloc(w / 2, h / 2)?;
    let quarter = alloc(w / 4, h / 4)?;
    let eighth = alloc(w / 8, h / 8)?;
    let out = alloc(w, h)?;

    let mut src = snap.dmabuf().clone();
    let mut d2 = half.dmabuf.clone();
    let mut d4 = quarter.dmabuf.clone();
    let mut d8 = eighth.dmabuf.clone();
    let mut dout = out.dmabuf.clone();

    blit(gles, &mut src, &mut d2, rect(w, h), rect(w / 2, h / 2))?;
    blit(gles, &mut d2, &mut d4, rect(w / 2, h / 2), rect(w / 4, h / 4))?;
    blit(gles, &mut d4, &mut d8, rect(w / 4, h / 4), rect(w / 8, h / 8))?;
    blit(gles, &mut d8, &mut dout, rect(w / 8, h / 8), rect(w, h))?;

    Some(out)
}

//! The wizard "electricity" effect — pure geometry + colour against the boundary in
//! `abi.rs`. No y5 / smithay / compositor types anywhere: the host hands in
//! projected window rects, this returns the bars to composite.

use compositor_artifact_plugin_abi_base::abi::{
    AbiDmabufQuad, AbiQuad, AbiRect, ArtifactMod, ArtifactMod_Ref, ArtifactPlugin,
    ArtifactPluginBox, ArtifactPlugin_TO, DmabufCtx, FrameCtx,
};
use abi_stable::export_root_module;
use abi_stable::prefix_type::PrefixTypeTrait;
use abi_stable::sabi_trait::TD_Opaque;
use abi_stable::std_types::RVec;

/// Band thickness (physical px) of the electricity frame.
const BAND: i32 = 8;

/// Plugin state: the flicker clock + the plugin's OWN bevy renderer (lazy).
struct Wizard {
    phase: f32,
    renderer: Option<crate::render::HatRenderer>,
    render_failed: bool,
    commit: u64,
}

impl ArtifactPlugin for Wizard {
    fn draw(&mut self, ctx: &FrameCtx) -> RVec<AbiQuad> {
        self.phase += ctx.dt;
        let flick = 0.55 + 0.45 * (self.phase * 9.0).sin().abs();
        let color = [0.25 * flick, 0.55 * flick, 1.0 * flick, 0.9];
        let mut out = RVec::new();
        for r in ctx.windows.iter() {
            for bar in perimeter_bars(*r) {
                // 410 = above all canvas content (windows + floating panes), under
                // the compositor's own screen UI.
                out.push(AbiQuad { rect: bar, color, band: 410 });
            }
        }
        out
    }

    /// Two spinning-hat quads sharing ONE plugin-rendered dmabuf: world (0,0) and
    /// screen top-right. The buffer is allocated with the HOST-NEGOTIATED format
    /// from `ctx` (zero-copy import guaranteed by construction).
    fn draw_dmabuf(&mut self, ctx: &DmabufCtx) -> RVec<AbiDmabufQuad> {
        if self.render_failed {
            return RVec::new();
        }
        if self.renderer.is_none() {
            let modifiers: Vec<u64> = ctx.modifiers.iter().copied().collect();
            match crate::render::HatRenderer::new(ctx.fourcc, &modifiers) {
                Ok(r) => self.renderer = Some(r),
                Err(e) => {
                    eprintln!("wizard plugin: renderer init failed: {e}");
                    self.render_failed = true;
                    return RVec::new();
                }
            }
        }
        let r = self.renderer.as_mut().expect("renderer");
        r.frame();
        self.commit += 1;

        const HAT: f64 = 180.0;
        const MARGIN: f64 = 24.0;
        let base = AbiDmabufQuad {
            id: 1,
            commit: self.commit,
            fd: r.raw_fd(),
            width: crate::render::BUFFER_PX as i32,
            height: crate::render::BUFFER_PX as i32,
            fourcc: r.fourcc,
            modifier: r.modifier,
            stride: r.stride,
            offset: r.offset,
            screen: 0,
            x: 0.0,
            y: 0.0,
            w: HAT,
            h: HAT,
            band: 410, // above canvas content
        };
        let mut out = RVec::new();
        out.push(base); // world (0,0)
        out.push(AbiDmabufQuad {
            id: 2,
            screen: 1,
            x: ctx.screen_w - HAT - MARGIN,
            y: MARGIN,
            band: 600, // above compositor screen UI, under the pointer
            ..base
        });
        out
    }
}

/// Four band bars (top, bottom, left, right) framing a projected window rect.
fn perimeter_bars(r: AbiRect) -> [AbiRect; 4] {
    let t = BAND;
    [
        AbiRect { x: r.x - t, y: r.y - t, w: r.w + 2 * t, h: t },
        AbiRect { x: r.x - t, y: r.y + r.h, w: r.w + 2 * t, h: t },
        AbiRect { x: r.x - t, y: r.y, w: t, h: r.h },
        AbiRect { x: r.x + r.w, y: r.y, w: t, h: r.h },
    ]
}

/// The exported ROOT MODULE (y5_api v2): abi_stable finds this and layout-verifies
/// it recursively before the host calls anything.
#[export_root_module]
fn get_root_module() -> ArtifactMod_Ref {
    ArtifactMod { new }.leak_into_prefix()
}

extern "C" fn new() -> ArtifactPluginBox {
    ArtifactPlugin_TO::from_value(
        Wizard { phase: 0.0, renderer: None, render_failed: false, commit: 0 },
        TD_Opaque,
    )
}

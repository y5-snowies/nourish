use compositor_background_two_draw_element::element::ParallaxBackground;
use compositor_background_two_state_base::state::Two;
use compositor_support_system_buffer_token_base::y5_buffer;
use compositor_support_system_trait_system_base::base::{BufferCx, System, SystemCx, WorldBuilder};
use compositor_support_system_world_frame_base::base::{self as layer, FramePlan, FrameTick};
use smithay::backend::renderer::gles::GlesRenderer;
use std::any::Any;

// The per-world background slot tokens live in `two.storage`; the system reads
// and writes that slot in update/draw/buffer.
use compositor_background_two_storage_base::base::{BG_TWO, BG_TWO_MUT};

enum TwoCmd {
    SetInstance(ParallaxBackground),
    Tick,
    Pan(f32, f32),
    Zoom(f32),
    /// New output size — applied IN PLACE (keeps `start_time`/`commit`), never a
    /// recreate. See the size-change note in `update()`.
    Resize(f32, f32),
}
y5_buffer!(TWO_BUF: TwoCmd);

/// The 2D parallax background system: `update()` (re)builds the GPU resource
/// via the platform hatch and ticks animation; `draw()` emits the node.
#[derive(Default)]
pub struct TwoSystem;

impl System for TwoSystem {
    fn name(&self) -> &'static str {
        "background.two"
    }

    fn register(&mut self, builder: &mut WorldBuilder) {
        builder.storage.insert(&BG_TWO, Two::new());
        builder.receive(&compositor_y5_camera_system_base::base::CAMERA_MOVED, Self::on_camera_moved);
        builder.receive(&compositor_y5_camera_system_base::base::CAMERA_ZOOMED, Self::on_camera_zoomed);
    }

    fn update(&mut self, cx: &mut SystemCx, _tick: &FrameTick) {
        // bevy lock-morph gate; absent BG_THREE (test worlds) = not locked.
        if cx.storage.try_get(&compositor_background_three_system_base::base::BG_THREE)
            .is_some_and(|b| b.example_lock_done) { return; }
        // Physical output size from the per-frame screen driver-data (set by the
        // frame driver before systems run) — no background-private size token.
        let size = cx.kernel.get(&compositor_orchestration_smithay_data_base::data::SCREEN).size;
        let size = (size.w as f32, size.h as f32);
        let state = cx.storage.get(&BG_TWO);
        // A size change must NOT recreate the instance. With multiple outputs of
        // differing sizes this `update()` runs once PER OUTPUT, each with that
        // output's `SCREEN` size, so a size-triggered rebuild would fire every
        // frame — resetting `start_time` (freezing the shader clock at ~0) and the
        // `commit` counter (no damage → the per-frame reschedule dies and the
        // parallax stops animating). Only a MISSING instance forces a full rebuild
        // (shader/params edits null the slot from the rim); a size change resizes
        // IN PLACE below (the shader is size-independent — `build()` ignores size,
        // and `draw()`/`bind_pane` use the actual per-pane `dst` size).
        let stale = state.instance.as_ref().is_some_and(|i| i.output_size != size);
        let rebuild = state.instance.is_none();
        // Resolve once: this world's override → preference default → built-in.
        // (Setting `instance = None` from the rim forces a rebuild on change.)
        let override_sel = state.background_shader.clone();
        let params = state.params.clone();
        if rebuild {
            if let Some(renderer) = cx
                .platform
                .as_deref_mut()
                .and_then(|p| p.downcast_mut::<compositor_orchestration_draw_platform_base::platform::Platform>())
                .and_then(|p| p.renderer())
            {
                let sel = override_sel.or_else(
                    compositor_developer_stats_registry_base::base::background_shader_default,
                );
                let instance = ParallaxBackground::new(renderer, size, sel.as_deref(), &params);
                cx.write(&TWO_BUF, TwoCmd::SetInstance(instance));
            }
            return;
        }
        // Keep the instance's own size current (used by the non-pane overview /
        // full-screen draw's `geometry()` damage rect) WITHOUT recreating it — each
        // output's `update()`+draw run in the same prepare, so this hands the frame
        // its output's size while the animation clock and commit counter survive.
        if stale {
            cx.write(&TWO_BUF, TwoCmd::Resize(size.0, size.1));
        }
        // Advance the parallax animation (mutation -> buffer, honoring read-only update).
        cx.write(&TWO_BUF, TwoCmd::Tick);
    }

    fn draw(&mut self, cx: &mut SystemCx, plan: &mut FramePlan) {
        if cx.storage.try_get(&compositor_background_three_system_base::base::BG_THREE)
            .is_some_and(|b| b.example_lock_done) { return; }
        // Renderer-agnostic node; the frame driver bridges + lowers it.
        if let Some(instance) = &cx.storage.get(&BG_TWO).instance {
            plan.push(layer::BACKGROUND, Box::new(instance.clone()));
        }
    }

    fn buffer(&mut self, cx: &mut BufferCx, message: Box<dyn Any>) {
        let two = cx.storage.get_mut(&BG_TWO_MUT);
        match *message.downcast::<TwoCmd>().expect("two buffer type") {
            TwoCmd::SetInstance(instance) => {
                two.shader_error = instance.shader_error.clone();
                two.instance = Some(instance);
            }
            TwoCmd::Tick => { if let Some(i) = &mut two.instance { i.update(); } }
            TwoCmd::Pan(x, y) => { if let Some(i) = &mut two.instance { i.pan = (x, y); } }
            TwoCmd::Zoom(z) => { if let Some(i) = &mut two.instance { i.zoom = z; } }
            TwoCmd::Resize(w, h) => { if let Some(i) = &mut two.instance { i.output_size = (w, h); } }
        }
    }

    /// Persist this world's background selection + variable overrides into a
    /// single per-world file `<world>/world.background.json`, rehydrated into the
    /// `BG_TWO` slot at world build.
    fn persist(
        &self,
    ) -> &'static [&'static compositor_support_system_persist_entry_base::base::PersistEntry] {
        compositor_background_two_storage_base::base::BACKGROUND_PERSISTS
    }
}

impl TwoSystem {
    fn on_camera_moved(&mut self, cx: &mut SystemCx, event: &compositor_y5_camera_system_base::base::CameraMoved) {
        cx.write(&TWO_BUF, TwoCmd::Pan(event.x as f32, event.y as f32));
    }

    fn on_camera_zoomed(&mut self, cx: &mut SystemCx, event: &compositor_y5_camera_system_base::base::CameraZoomed) {
        cx.write(&TWO_BUF, TwoCmd::Zoom(event.zoom as f32));
    }
}

use smithay::{
    backend::renderer::gles::GlesRenderer,
    utils::{Physical, Point, Size},
};
use compositor_support_bevy_core_compositor_base::BevyHandle;
use compositor_background_three_lock_scene::{MorphCommand, MorphScene};
use compositor_orchestration_core_state_base::Loop;
use compositor_y5_lock_state_base::state::LockActiveCapture;

/// Build the lock-screen morph instance while the ORIGINATING session is still
/// on screen (this may run during `pending`). The plane samples the LIVE output
/// capture, so while it warms up (cast import + first render) it mirrors the
/// still-visible desktop from ON TOP — there is never a frame where the session
/// is gone but the plane is not yet up. The fold itself is NOT started here (see
/// [`start_fold`]); the plane sits flat over the session until the session scene
/// is dropped at the `pending`→done handoff, which naturally freezes the last
/// captured frame in place for the fold. Returns the bevy handle once created.
pub(crate) fn create(
    state: &mut Loop,
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> Option<BevyHandle<MorphScene>> {
    // The live capture must already exist (armed a frame earlier in
    // `lock_visual`). Keep it LIVE — do NOT freeze it to a `Snapshot` — so the
    // plane tracks the desktop until the session scene stops rendering at
    // handoff, which freezes the last frame in place for the fold.
    let capture = match &state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().get_mut(&compositor_y5_lock_system_base::base::LOCK_MUT).active {
        Some(active) => match &active.capture {
            LockActiveCapture::Capture(handle) => handle.clone(),
            _ => return None,
        },
        None => abort!("locked"),
    };
    let wgpu = capture.wgpu_texture()?;

    let gpu = state.inner.environment.GPU.clone();

    // The LOCK world OWNS its bevy registry, pre-created at startup by the loader
    // prewarm pass — asserted present here rather than built mid-render.
    let Some(registry) = state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT).and_then(|b| b.registry.as_mut()) else {
        abort!("lock: bevy registry missing — startup prewarm failed");
    };

    // Create one lock-screen morph instance at full output size, in screen
    // space so it stays fixed regardless of camera transform.
    match registry.create_screen(
        &gpu.as_str(),
        MorphScene::new((size.w as u32, size.h as u32), wgpu),
        renderer,
        Point::from((0, 0)),
        size,
        compositor_orchestration_draw_layer_base::base::Layer::LOCK_SCENE.bits(),
    ) {
        Ok(handle) => Some(handle),
        Err(_) => None,
    }
}

/// Dispatch the morph fold. Called once at the moment the originating session
/// scene is dropped (`!pending`): the warmed-up snapshot plane is already
/// covering the screen, so the fold begins seamlessly.
pub(crate) fn start_fold(state: &mut Loop, handle: BevyHandle<MorphScene>) {
    let Some(registry) = state.inner.worlds.get_mut(compositor_y5_lock_system_base::base::LOCK_WORLD).storage_mut().try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT).and_then(|b| b.registry.as_mut()) else {
        return;
    };
    if let Err(e) = registry.dispatch_command(handle, MorphCommand::Lock) {
        error!("dispatch lock command failed: {}", e);
    }
}

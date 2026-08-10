use crate::surface;
use crate::three;
use compositor_y5_lock_scene_element::element::LockSceneElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{ImportAll, ImportDma, ImportMem, Renderer, Texture};
use smithay::utils::{Physical, Point, Scale, Size};
use compositor_orchestration_draw_dispatch_frame::SceneDispatch;
use compositor_orchestration_draw_scene_element::element::PreImported;
use compositor_orchestration_core_state_base::Loop;

pub struct Scene<R: Renderer> {
    pub Element: Vec<LockSceneElement<R>>,
}

/// GLES-built lock elements carried from `prepare()` into `scene()` (same split
/// as the main scene path: the iced lock surface + bevy lock background render
/// into GLES resources every frame).
pub struct LockPrepared {
    pub surfaces: Vec<compositor_monitor_compositor_iced_base::IcedRenderElement>,
    pub three: Vec<compositor_support_bevy_core_compositor_base::BevyRenderElement>,
    pub background_two: Option<compositor_background_two_draw_element::element::ParallaxBackground>,
}

/// Active-monitor-only gate. The lock UI (auth panel + morph background + pointer)
/// is built and rendered ONLY on the active output; other outputs keep just the
/// parallax. This sizes/centers the auth panel once for the active monitor and
/// keeps the capture on it. `render_output == None` = winit/single-output pass →
/// active (that path is unchanged). Mirrors the overview overlay's convention.
fn on_active_output(s: &Loop) -> bool {
    s.inner
        .render_output
        .as_ref()
        .is_none_or(|k| *k == s.inner.active_output_key())
}

/// GLES preparation phase for the lock scene.
pub fn prepare(state: &mut Loop, renderer: &mut GlesRenderer, size: Size<i32, Physical>) -> LockPrepared {
    let active = on_active_output(state);

    // Lock VISUAL + auth-message drain + iced/bevy elements: active monitor only,
    // so the auth panel is built/centered once for that output's size and the
    // capture targets it. Other outputs fall through to parallax-only below.
    let (surfaces, three) = if active {
        // Build the lock VISUAL lazily now that a renderer exists (the logical lock
        // engaged earlier, possibly while dark). No-op once built.
        compositor_y5_lock_interface_base::interface::lock_visual(state, renderer, size);
        compositor_y5_lock_scene_hook::hook::hook(state, renderer, size);
        (
            surface::scene(state, renderer, size),
            three::scene(state, renderer, size),
        )
    } else {
        (vec![], vec![])
    };

    let compositor_orchestration_core_state_base::state::Status::Locked { pending, .. } = state.inner.status
    else {
        abort!();
    };
    // Render two background only when not pending, otherwise it's already
    // rendered by the regular scene.
    // See `ParallaxBackground::bind_overlay` — the lock screen builds its own
    // plan, so nothing else supplies the pane, refresh or frame serial.
    let background_two = if !pending {
        compositor_background_two_draw_scene::scene::scene(state).map(|mut b| {
            let out: std::sync::Arc<str> =
                std::sync::Arc::from(state.inner.current_output_key().as_str());
            if let Some(w) = b.world.filter(|w| state.inner.worlds.contains(*w)) {
                b.bind_frame(compositor_pipeline_world_system_base::base::frame(state.inner.worlds.get(w).storage(), &out));
            }
            b.bind_overlay(
                "lock",
                &out,
                state.inner.current_refresh(),
                state.inner.next_frame_serial(),
            );
            b
        })
    } else {
        None
    };

    // Same per-frame statement the orchestration scene makes, for the same reason
    // — the frame driver picks one prepare path, and when the lock screen owns the
    // frame the orchestration one never runs, so nothing else would state the
    // facts of the background actually on screen.
    //
    // ONLY when this path supplies the background. While `pending`, the regular
    // scene is still drawing it and has already published; publishing the `None`
    // above there would overwrite a live world's answers with the neutral ones and
    // stop the engine collecting a set its bundle needs.
    if !pending {
        if let Some(w) = background_two.as_ref().and_then(|b| b.world) {
            if state.inner.worlds.contains(w) {
                compositor_pipeline_world_system_base::base::publish_facts(
                    state.inner.worlds.get_mut(w).storage_mut(),
                    background_two.as_ref().and_then(|b| b.pipeline.as_deref()),
                );
            }
        }
    }

    LockPrepared {
        surfaces,
        three,
        background_two,
    }
}

pub fn scene<R>(
    state: &mut Loop,
    renderer: &mut R,
    size: Size<i32, Physical>,
    prepared: LockPrepared,
) -> Scene<R>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let mut elements: Vec<LockSceneElement<R>> = Vec::new();

    let compositor_orchestration_core_state_base::state::Status::Locked { pending, .. } = state.inner.status
    else {
        abort!();
    };

    // Pointer only on the active monitor (where the auth panel lives).
    if !pending && on_active_output(state) {
        let pointer = compositor_orchestration_seat_pointer_draw::scene::element(state, renderer, size);
        elements.extend(pointer.into_iter().map(LockSceneElement::Pointer));
    }

    // iced/bevy: on renderers that prefer dmabuf (Vulkan), import their dmabuf
    // into a renderer-native texture (PreImported); on GLES keep the welded
    // Surface/Background3D elements. (Same as the main scene's stage-4 routing —
    // without this the lock UI is blank on Vulkan.)
    for e in prepared.surfaces {
        if let Some(el) = iced_to_lock(renderer, e) {
            elements.push(el);
        }
    }
    for e in prepared.three {
        if let Some(el) = bevy_to_lock(renderer, e) {
            elements.push(el);
        }
    }
    elements.extend(prepared.background_two.map(LockSceneElement::Background2D));

    Scene { Element: elements }
}

fn iced_to_lock<R>(
    renderer: &mut R,
    e: compositor_monitor_compositor_iced_base::IcedRenderElement,
) -> Option<LockSceneElement<R>>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    if !R::prefers_dmabuf() {
        return Some(LockSceneElement::Surface(e));
    }
    match renderer.import_dmabuf(&e.dmabuf, None) {
        Ok(texture) => Some(LockSceneElement::Texture(PreImported {
            texture,
            location: e.location,
            size: e.size,
            world_zoom: e.world_zoom,
            id: e.id,
            commit: e.commit_counter,
        })),
        Err(err) => {
            error!("lock scene: import of iced dmabuf failed: {err}");
            None
        }
    }
}

fn bevy_to_lock<R>(
    renderer: &mut R,
    e: compositor_support_bevy_core_compositor_base::BevyRenderElement,
) -> Option<LockSceneElement<R>>
where
    R: Renderer + ImportAll + ImportDma + ImportMem + SceneDispatch,
    R::TextureId: Texture + Clone + Send + 'static,
{
    // `texture.is_some()` = an inline instance with its own GLES view. An
    // off-thread (ring-worker) one has none, and drawing it would be a silent
    // no-op — import the dmabuf instead. See `draw.node`'s Background3D arm:
    // winit's lock pass is GLES even when the scene pass is Vulkan.
    if !R::prefers_dmabuf() && e.texture.is_some() {
        return Some(LockSceneElement::Background3D(e));
    }
    match renderer.import_dmabuf(&e.dmabuf, None) {
        Ok(texture) => Some(LockSceneElement::Texture(PreImported {
            texture,
            location: e.location,
            size: e.size,
            world_zoom: e.world_zoom,
            id: e.id,
            commit: e.commit_counter,
        })),
        Err(err) => {
            error!("lock scene: import of bevy dmabuf failed: {err}");
            None
        }
    }
}

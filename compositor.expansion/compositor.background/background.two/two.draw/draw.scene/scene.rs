use compositor_background_two_draw_element::element::ParallaxBackground;
use compositor_orchestration_core_state_base::Loop;

pub fn scene(state: &mut Loop) -> Option<ParallaxBackground> {
    let mut backgrounds: Option<ParallaxBackground> = None;
    // Focused world (== the spatial world behind any lock overlay; switch() only
    // moves `active`, not `spawn_target`). try_get_mut: a world without the bevy
    // ThreeSystem has no BG_THREE — treat that as "not locked-morphing".
    let target = state.inner.worlds.spawn_target();
    let lock_morph = state.inner.worlds.get_mut(target).storage_mut()
        .try_get_mut(&compositor_background_three_system_base::base::BG_THREE_MUT)
        .is_some_and(|b| b.example_lock_done);
    if lock_morph {
        return None;
    }

    if let Some(ref mut background) = state.inner.worlds.get_mut(target).storage_mut().get_mut(&compositor_background_two_storage_base::base::BG_TWO_MUT).instance {
        background.update();

        backgrounds = Some(background.clone());

        // CHECK: Requires more delitful care in the shader background.
        state.schedule_redraw_post_vblank();
    }

    backgrounds
}

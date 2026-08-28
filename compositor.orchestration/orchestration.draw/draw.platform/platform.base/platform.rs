//! `Platform` — the live-smithay hatch handed to systems via `SystemCx.platform`
//! (`Option<&mut dyn Any>`). Because `dyn Any: 'static`, the hatch cannot carry a
//! borrowing struct directly; `Platform` therefore stores raw pointers and is
//! itself `'static`, exposing the live `&mut` through safe accessors.
//!
//! SAFETY CONTRACT (upheld by the frame/input driver): `Platform` lives only for
//! the duration of one `world.update/draw/input` call; the driver holds `&mut`
//! borrows of the pointed-to `Loop` fields (renderer, `state.inner.space_state()`) for
//! that whole call and touches neither itself, so the pointers stay valid and
//! unaliased while systems use them.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::{Space, Window};

pub struct Platform {
    renderer: *mut GlesRenderer,
    space: *mut Space<Window>,
    /// The DRM render node the renderer composites on — what iced registries
    /// key their GPU resources by. Travels with the renderer it describes.
    render_node: String,
}

impl Platform {
    /// Build from live borrows. SAFETY: the borrows must outlive this `Platform`
    /// (the driver scopes it to a single system-dispatch call — see module docs).
    pub unsafe fn new(renderer: Option<&mut GlesRenderer>, space: &mut Space<Window>, render_node: &str) -> Self {
        Self {
            renderer: renderer.map_or(std::ptr::null_mut(), |r| r as *mut GlesRenderer),
            space: space as *mut Space<Window>,
            render_node: render_node.to_owned(),
        }
    }

    /// The render node `renderer()` composites on.
    pub fn render_node(&self) -> &str {
        &self.render_node
    }

    /// The active GLES renderer, if this is a GLES phase.
    pub fn renderer(&mut self) -> Option<&mut GlesRenderer> {
        if self.renderer.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.renderer })
        }
    }

    /// The window space (geometry queries, `map_element`, element geometry).
    pub fn space(&mut self) -> &mut Space<Window> {
        unsafe { &mut *self.space }
    }
}

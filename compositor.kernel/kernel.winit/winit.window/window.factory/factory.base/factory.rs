//! Winit window/backend construction + the dev output. (Ex winit wire.rs
//! `new()`, moved.)

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self as WinitBackend, WinitEventLoop, WinitGraphicsBackend};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::utils::Transform;

pub struct WinitWindow {
    pub output: Output,
    pub mode: Mode,
    pub winit_backend: WinitGraphicsBackend<GlesRenderer>,
    pub winit_loop: WinitEventLoop,
}

pub fn create() -> Result<WinitWindow, String> {
    info!("Init winit backend");

    let (backend, winit) = WinitBackend::init::<GlesRenderer>()
        .map_err(|e| format!("winit init failed: {e:?}"))?;

    // Hide the host OS cursor: y5 draws (and pane-clamps) its own cursor, so the
    // unconfined host cursor would otherwise cross viewport edges under nesting.
    backend.window().set_cursor_visible(false);

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };
    info!(
        "winit backend OK: window {}x{} @ {}mHz",
        mode.size.w, mode.size.h, mode.refresh
    );

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "Winit".into(),
            serial_number: "Unknown".into(),
        },
    );

    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    info!("winit output 'winit' configured (transform Flipped180)");

    Ok(WinitWindow {
        output,
        mode,
        winit_backend: backend,
        winit_loop: winit,
    })
}

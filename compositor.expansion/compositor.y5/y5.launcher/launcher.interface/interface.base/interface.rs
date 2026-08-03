use crate::listing_xdg_basic;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Size};
use std::sync::mpsc::Sender;
use compositor_orchestration_core_state_base::Loop;
use compositor_monitor_compositor_iced_base::HandleId;
use compositor_monitor_launcher_ui_base::{Application, LauncherMessage};
use compositor_y5_surface_protocol_base::launcher;
use compositor_y5_surface_protocol_base::launcher::message::InternalAction;
use compositor_y5_surface_protocol_base::protocol::SurfaceMessageType::Launcher;
use compositor_y5_surface_protocol_base::protocol::{SurfaceMessage, SurfaceMessageType};

pub fn start(_loop: &mut Loop, renderer: &mut GlesRenderer) {
    if _loop.inner.launcher_mut().handle.is_some() {
        return;
    }

    // The ACTIVE monitor (cursor's output) — the launcher opens on the monitor the
    // user is on, sized for it, not the primary.
    let output = _loop.inner.active_output();
    // let output_geom_i32 = _loop
    //     .state
    //     .space
    //     .state
    //     .output_geometry(output)
    //     .expect("output has geometry");
    // let screen_size: Size<f64, Logical> = output_geom_i32.size.to_f64();
    //
    // let width = 800;
    // let height = 800;
    // let x = ((screen_size.w / 2.0) - (width as f64 / 2.0)).round() as i32;
    // let y = ((screen_size.h / 2.0) - (height as f64 / 2.0)).round() as i32;

    let mode = output.current_mode().unwrap_or_else(|| abort!("output has mode"));
    let screen_size_physical = mode.size; // Size<i32, Physical>

    let width = 800;
    let height = 800;
    let x = (screen_size_physical.w / 2 - width / 2);
    let y = (screen_size_physical.h / 2 - height / 2);


    let handle = compositor_y5_surface_draw_handle::handle::load(
        _loop,
        renderer,
        compositor_monitor_launcher_ui_base::Launcher::new(listing_xdg_basic::load_applications()),
        Rectangle::new(Point::new(x, y), Size::new(width, height)),
        compositor_y5_surface_draw_handle::handle::IcedSpace::Screen,
        compositor_orchestration_draw_layer_base::base::Layer::SCENE.bits(),
    );

    _loop.inner.launcher_mut().handle = Some(handle);

    // Give the launcher iced keyboard focus immediately so typing reaches the search
    // field from the moment it opens — both physical keys AND the on-screen keyboard's
    // injected keys (which route to the focused iced surface). Without this, opening the
    // launcher + OSK together (from the touch menu) would leave OSK taps going nowhere
    // until a physical key first set focus.
    if let Some(reg) = _loop.inner.surface_mut().registry.as_mut() {
        reg.set_keyboard_focus(Some(handle.id));
    }
    let tx = _loop.inner.surface_mut().surface_message_buffer_channel.0.clone();
    _loop.inner.surface_mut()
        .registry
        .as_mut()
        .unwrap()
        .set_message_handler(handle, move |message: &LauncherMessage| __dispatch(message, &tx));
}

fn __dispatch(p1: &LauncherMessage, p2: &Sender<SurfaceMessage>) {
    match p1 {
        // Relevant
        LauncherMessage::Launch {
            direction,
            id,
            bin,
            args,
        } => {
            p2.send(SurfaceMessage {
                message: SurfaceMessageType::Launcher(launcher::message::LauncherMessage {
                    message: launcher::message::Source::External(
                        launcher::message::ExternalAction::Start {
                            args: args.clone(),
                            direction: direction.clone(),
                            bin: bin.clone(),
                            id: id.clone(),
                        },
                    ),
                }),
            });
        }
        // Relevant
        LauncherMessage::Exit => {
            p2.send(SurfaceMessage {
                message: SurfaceMessageType::Launcher(launcher::message::LauncherMessage {
                    message: launcher::message::Source::External(
                        launcher::message::ExternalAction::Exit,
                    ),
                }),
            });
        }
        // Irrelevant
        // LauncherMessage::Event(_) => {}
        // Irrelevant
        // LauncherMessage::Tick => {}
        // Irrelevant
        // LauncherMessage::SetApps(_) => {}
        // LauncherMessage::MoveCursor(_) => {}
        // LauncherMessage::FocusSelection => {}
        // LauncherMessage::UnfocusSelection => {}
        // LauncherMessage::ClearQuery => {}
        // LauncherMessage::Backspace => {}
        // LauncherMessage::AppendText(_) => {}
        _ => {}
    }
}

/// Whether the launcher surface is currently open.
pub fn is_open(_loop: &Loop) -> bool {
    _loop.inner.launcher().handle.is_some()
}

/// Tear down the launcher surface (if open). Mirrors the `Exit` reducer path.
pub fn close(_loop: &mut Loop) {
    let Some(handle) = _loop.inner.launcher_mut().handle else {
        return;
    };
    if let Some(reg) = _loop.inner.surface_mut().registry.as_mut() {
        reg.destroy(handle);
    }
    _loop.inner.launcher_mut().handle = None;
}

/// Close the launcher when a touch/pen tap landed OUTSIDE its surface. `hit` is the
/// iced handle under the tap (`None` for a window/layer/empty hit). A tap ON the
/// launcher (`hit == launcher`) is left alone so a cell tap can launch. Returns
/// whether it closed (so the caller can swallow the dismissing tap). Wired only into
/// the touch-down / pen-tip paths, so a plain mouse click never dismisses it.
pub fn dismiss_if_outside(_loop: &mut Loop, hit: Option<HandleId>) -> bool {
    let Some(handle) = _loop.inner.launcher_mut().handle else {
        return false;
    };
    if hit == Some(handle.id) {
        return false;
    }
    // A tap on a keyboard-transparent companion overlay (the on-screen keyboard) is NOT
    // "outside": the OSK types INTO the launcher, so tapping its keys must not dismiss
    // it. Such surfaces never take keyboard focus, so they're never a real focus change.
    if let Some(h) = hit {
        if _loop
            .inner
            .surface()
            .registry
            .as_ref()
            .is_some_and(|r| r.is_keyboard_transparent(h))
        {
            return false;
        }
    }
    close(_loop);
    true
}

pub fn start_defered(p0: &mut Loop) {
    p0.inner.surface_mut()
        .surface_message_buffer_channel
        .0
        .send(SurfaceMessage {
            message: SurfaceMessageType::Launcher(launcher::message::LauncherMessage {
                message: launcher::message::Source::Internal(InternalAction::Start),
            }),
        });
}

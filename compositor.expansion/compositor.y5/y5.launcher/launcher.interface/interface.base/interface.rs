use crate::listing_xdg_basic;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Point, Rectangle, Size};
use std::sync::mpsc::Sender;
use compositor_orchestration_core_state_base::Loop;
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

    info!("{:?}", listing_xdg_basic::load_applications());
    // [Application { id: "glmark2", title: "Glmark2", bin: "glmark2", args: [], icon_path: Some("/usr/share/pixmaps/glmark2.png"), usage_count: 0, usage_time: None }, Application { id: "footclient", title: "Foot Client", bin: "footclient", args: [], icon_path: Some("/usr/share/icons/hicolor/scalable/apps/foot.svg"), usage_count: 0, usage_time: None }, Application { id: "glmark2-es2", title: "Glmark2-es2", bin: "glmark2-es2", args: [], icon_path: Some("/usr/share/pixmaps/glmark2-es2.png"), usage_count: 0, usage_time: None }, Application { id: "foot-server", title: "Foot Server", bin: "foot", args: ["--server"], icon_path: Some("/usr/share/icons/hicolor/scalable/apps/foot.svg"), usage_count: 0, usage_time: None }, Application { id: "xterm", title: "XTerm", bin: "xterm", args: [], icon_path: Some("/usr/share/icons/hicolor/scalable/apps/xterm-color.svg"), usage_count: 0, usage_time: None }, Application { id: "google-chrome", title: "Google Chrome", bin: "/usr/bin/google-chrome-stable", args: [], icon_path: Some("/usr/share/icons/hicolor/16x16/apps/google-chrome.png"), usage_count: 0, usage_time: None }, Application { id: "Alacritty", title: "Alacritty", bin: "alacritty", args: [], icon_path: Some("/usr/share/pixmaps/Alacritty.svg"), usage_count: 0, usage_time: None }, Application { id: "glmark2-es2-wayland", title: "Glmark2-es2-wayland", bin: "glmark2-es2-wayland", args: [], icon_path: Some("/usr/share/pixmaps/glmark2-es2-wayland.png"), usage_count: 0, usage_time: None }, Application { id: "foot", title: "Foot", bin: "foot", args: [], icon_path: Some("/usr/share/icons/hicolor/scalable/apps/foot.svg"), usage_count: 0, usage_time: None }, Application { id: "glmark2-wayland", title: "Glmark2-wayland", bin: "glmark2-wayland", args: [], icon_path: Some("/usr/share/pixmaps/glmark2-wayland.png"), usage_count: 0, usage_time: None }]
    _loop.inner.launcher_mut().handle = Some(handle);
    //
    let tx = _loop.inner.surface_mut().surface_message_buffer_channel.0.clone();
    _loop.inner.surface_mut()
        .registry
        .as_mut()
        .unwrap()
        .instance_mut(handle)
        .unwrap()
        .runtime_mut()
        .set_message_handler(move |message: &LauncherMessage| __dispatch(message, &tx));
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

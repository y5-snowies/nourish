//! The field-by-field interactive flow. Takes the starting values (an existing
//! file if present, else the template) and returns a fully-populated `Environment`.

use crate::drm_probe;
use crate::prompt::{ask, ask_u8, choose, yes_no};
use crate::select::{select_list, Item};
use crate::term::Nav;
use compositor_configurator_hardware_gpu_base::base::render_devices;
use compositor_model_environment_config_base::base as config;
use compositor_model_environment_config_base::base::{Environment, SCHEMA_VERSION};

/// The settings flow is a straight sequence of required fields — there is no
/// "back" target mid-flow, so no field (including the GPU list) offers Escape.
pub fn interactive(base: Environment) -> Environment {
    println!("y5.compositor.settings — every field is required; press Enter to keep the shown value.");
    let mut env = Environment {
        // Not prompted: the schema version this editor writes (migrations in
        // config.base lift older files on load).
        version: SCHEMA_VERSION,
        renderer: choose(
            "renderer",
            "Renderer backend. NOTE for AMD users: some have reported Vulkan not \
             working on AMD; if you're on an AMD card, choose 'gles'.",
            &["vulkan", "gles"],
            &base.renderer,
        ),
        renderer_fallback: yes_no(
            "renderer_fallback",
            "Fall back to GLES if Vulkan initialization fails.",
            base.renderer_fallback,
        ),
        // Prompted as "composite sync" with friendly labels. The stored field
        // stays `renderer_sync`, and the stored value for the default stays the
        // historical `"infence"` — `config.base` owns both spellings, and
        // "native" is only what it is CALLED here.
        renderer_sync: {
            let options = [
                format!("{} — hand the display the render fence (default)", config::RENDERER_SYNC_NATIVE),
                format!("{} — wait for the GPU on the CPU first", config::RENDERER_SYNC_SYNCHRONOUS),
            ];
            let refs: Vec<&str> = options.iter().map(String::as_str).collect();
            // Anything that is not the explicit opt-out is the default, so a file
            // carrying a dead spelling shows the default rather than mis-reporting
            // what the compositor will actually do with it.
            let current = match config::renderer_sync_fence(&base.renderer_sync) {
                true => refs[0],
                false => refs[1],
            };
            let picked = choose(
                "composite sync (renderer_sync)",
                "How the composited frame is committed to the display.",
                &refs,
                current,
            );
            match picked == refs[1] {
                true => config::RENDERER_SYNC_SYNCHRONOUS.to_string(),
                false => config::RENDERER_SYNC_DEFAULT.to_string(),
            }
        },
        // NOT prompted: the shipped `realtime` is the right value on every
        // machine — it degrades to the old `auto` behavior by itself when
        // CAP_SYS_NICE is missing — so there is nothing here for someone
        // installing to decide. Carried through rather than forced, so a value
        // hand-edited into the file survives a trip through this tool.
        priority: base.priority.clone(),
        // Experimental — always disabled, never prompted (hdr, vk_diag, and the
        // two window-sizing flags below are forced off regardless of the
        // existing file).
        hdr: false,
        depth: ask_u8(
            "depth",
            "Scanout bit depth: 8 (SDR) or 10 (deep color).",
            &[8, 10],
            base.depth,
        ),
        vrr: yes_no("vrr", "Enable adaptive sync / VRR.", base.vrr),
        render_node: select_render_node(&base.render_node),
        // Resolved after this literal, not here: the recommendation depends on the
        // render node picked on the line above, and struct fields cannot read each
        // other. Prompting it last also keeps every pre-existing prompt in place.
        scanout_node: String::new(),
        desktop_name: ask(
            "desktop_name",
            "XDG desktop name advertised to clients.",
            &base.desktop_name,
        ),
        log_level: ask(
            "log_level",
            "Developer-log level spec, e.g. info,warn,error.",
            &base.log_level,
        ),
        vk_diag: String::new(),
        capture_encoder: choose(
            "capture_encoder",
            "Hardware video-capture encoder (mesa/vaapi -> VAAPI, else NVENC).",
            &["nvenc", "vaapi", "mesa"],
            &base.capture_encoder,
        ),
        capture_codec: choose(
            "capture_codec",
            "Live capture codec (falls back av1 -> h265 -> h264 by availability).",
            &["av1", "h265", "h264"],
            &base.capture_codec,
        ),
        capture_quality: choose(
            "capture_quality",
            "Live capture quality: lossless (CQ 19) or optimized (smaller, higher CQ).",
            &["lossless", "optimized"],
            &base.capture_quality,
        ),
        capture_refresh_rate_max: choose(
            "capture_refresh_rate_max",
            "Max capture frame rate.",
            &["30", "60", "90", "120"],
            &base.capture_refresh_rate_max.to_string(),
        )
        .parse()
        .unwrap_or(60)
        .clamp(30, 120),
        capture_background_encoder: choose(
            "capture_background_encoder",
            "Auto background software re-encode after capture: ffmpeg, or off.",
            &["", "ffmpeg"],
            &base.capture_background_encoder,
        ),
        capture_nvenc_allow_readback_fallback: yes_no(
            "capture_nvenc_allow_readback_fallback",
            "Fall back to the slower readback encoder if NVENC zero-copy fails (else show an error).",
            base.capture_nvenc_allow_readback_fallback,
        ),
        capture_variable_frame_rate: yes_no(
            "capture_variable_frame_rate",
            "Keep variable frame rate (true) or force constant frame rate during re-encode (false).",
            base.capture_variable_frame_rate,
        ),
        // Experimental window-sizing flags — always disabled, never prompted.
        window_client_size_fallback: false,
        window_subsurface_shrinks: false,
    };
    env.scanout_node = select_scanout_node(&base.scanout_node, &env.render_node);
    env
}

/// Pick the scanout device. Empty — let the udev heuristic choose — is the answer on
/// every machine whose render and display engines are the same DRM device, so it leads
/// the list and is what the recommendation says by default.
///
/// It stops being the answer when they are SEPARATE devices, because the heuristic
/// smithay uses ranks "has a render node" above "has connectors" (`primary_gpu`, 2nd
/// priority), so it picks the render-only device and assembly dies probing a card with
/// no CRTCs. A Raspberry Pi is the standard case. So: when the card holding
/// `render_node` is NOT among the cards that actually have a connected monitor, the
/// heuristic is going to be wrong here and the recommendation moves to a card that
/// provably can scan out.
///
/// Probing is read-only and needs no DRM master, but it CAN come back empty (no
/// permission, no `/dev/dri`, headless install) — then this degrades to a free-text
/// prompt rather than pretending to know, exactly as `select_render_node` does.
fn select_scanout_node(current: &str, render_node: &str) -> String {
    let monitors = drm_probe::probe();
    let cards = drm_probe::scanout_candidates(&monitors);
    if cards.is_empty() {
        return ask(
            "scanout_node",
            "DRM device to scan out on. Empty = pick it automatically (recommended).",
            current,
        );
    }

    // Can the heuristic reach a card with a monitor on it from the chosen render node?
    let render_card_scans_out = cards.iter().any(|c| drm_probe::same_device(render_node, c));
    let recommended: Option<&str> = match render_card_scans_out {
        true => None,                                 // empty is right
        false => cards.first().map(String::as_str),   // name a card that works
    };

    let auto_detail = match recommended {
        None => "recommended — render and display are the same device".to_string(),
        Some(_) => format!("would pick the wrong device on this machine ({render_node} has no outputs)"),
    };
    let mut items = vec![Item::new("Automatic (detect)", auto_detail)];
    for card in &cards {
        let outputs: Vec<&str> = monitors
            .iter()
            .filter(|m| &m.node == card)
            .map(|m| m.label.as_str())
            .collect();
        let detail = match recommended == Some(card.as_str()) {
            true => format!("{card} — recommended: {}", outputs.join(", ")),
            false => format!("{card} — {}", outputs.join(", ")),
        };
        items.push(Item::new(format!("Scan out on {card}"), detail));
    }

    // Mark what the file currently holds, so Enter keeps it; when it holds nothing,
    // mark whatever is recommended so Enter takes the advice.
    let marked = match cards.iter().position(|c| c == current) {
        Some(i) => i + 1,
        None if current.is_empty() => recommended
            .and_then(|r| cards.iter().position(|c| c == r))
            .map_or(0, |i| i + 1),
        None => 0,
    };

    match select_list("Scanout device (display)", &items, Some(marked), false) {
        Nav::Selected(0) => String::new(),
        Nav::Selected(i) => cards[i - 1].clone(),
        Nav::Back => current.to_string(),
    }
}

/// Pick the DRM render node from a list of detected GPUs with estimated card names —
/// the SAME enumeration + naming the in-compositor settings window uses (shared by
/// path via `gpu.base`). Falls back to a free-text prompt when no render nodes are
/// present (headless / no `/dev/dri`), so the tool still works there. Like the other
/// fields, the list offers no Escape (`allow_back = false`); EOF/non-TTY keeps the
/// current value.
fn select_render_node(current: &str) -> String {
    let devs = render_devices();
    if devs.is_empty() {
        return ask("render_node", "DRM render node path.", current);
    }
    let items: Vec<Item> = devs.iter().map(|d| Item::new(d.name.clone(), d.node.clone())).collect();
    let cur = devs.iter().position(|d| d.node == current);
    match select_list("Render device (GPU)", &items, cur, false) {
        Nav::Selected(i) => devs[i].node.clone(),
        Nav::Back => current.to_string(),
    }
}

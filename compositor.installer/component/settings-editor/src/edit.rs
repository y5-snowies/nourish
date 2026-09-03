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
            "Renderer backend. Vulkan is the default and is right for current hardware; \
             'gles' is the fail-safe for older GPUs.",
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
    };
    env.scanout_node = select_scanout_node(&base.scanout_node, &env.render_node);
    env
}

/// Pick the scanout device. Empty — let the compositor choose — is ALWAYS the
/// recommendation now, including where the render and display engines are separate
/// devices.
///
/// That is a change. It used to recommend an explicit card on a split-device machine,
/// because smithay's `primary_gpu` heuristic ranks "has a render node" above "has
/// connectors" and so picks the render-only half, and assembly then died on a card with
/// no CRTCs. The compositor now corrects that itself: `assemble.display` probes the
/// heuristic's pick and moves to a device with a CONNECTED monitor when the pick has
/// none. So the old advice ("automatic would pick the wrong device here") is no longer
/// true, and telling a user to pin a card they do not need is worse than silence — a
/// pinned path is one more thing to be wrong after a kernel or cable change.
///
/// The explicit entries stay, as an override for when the automatic choice is wrong on
/// hardware we have not seen. They are offered, not advised.
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

    // Whether the render node's own card has a monitor is still worth SAYING — it tells the
    // user their machine is a split-device one — but it no longer changes the advice.
    let split = !cards.iter().any(|c| drm_probe::same_device(render_node, c));
    let auto_detail = match split {
        false => "recommended — render and display are the same device".to_string(),
        true => format!("recommended — {render_node} has no outputs, so the compositor \
                         picks a display device on its own"),
    };
    let mut items = vec![Item::new("Automatic (detect)", auto_detail)];
    for card in &cards {
        let outputs: Vec<&str> = monitors
            .iter()
            .filter(|m| &m.node == card)
            .map(|m| m.label.as_str())
            .collect();
        items.push(Item::new(
            format!("Scan out on {card}"),
            format!("{card} — {}", outputs.join(", ")),
        ));
    }

    // Mark what the file holds so Enter keeps it; an empty file marks Automatic, which is
    // now the recommendation in every case.
    let marked = cards.iter().position(|c| c == current).map_or(0, |i| i + 1);

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

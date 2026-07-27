#!/usr/bin/env bash
# Assemble the compositor's settings JSON and write it to the settings file.
#
# The compositor reads ALL of its own configuration from one file,
# ~/.config/y5.compositor/settings.json (a JSON object with every field REQUIRED;
# it panics at startup if the file is missing or any field is absent). This is the
# one place the dev-loop scripts turn the individual COMPOSITOR_* knobs (and their
# historical defaults) into that file.
#
# Source this file, then call `compositor_write_settings` before launching the
# compositor (it creates the file at $XDG_CONFIG_HOME/y5.compositor/settings.json,
# falling back to $HOME/.config). `compositor_env_json` still emits the raw JSON for
# callers that need it host-side (e.g. the udev guest writes it into the VM).

# Map common truthy spellings → JSON `true` / `false`.
_y5_bool() {
    case "$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')" in
        1 | true | yes | on | gles) printf 'true' ;;
        *) printf 'false' ;;
    esac
}

# Emit the complete settings JSON from the individual env knobs, applying the
# compositor's historical defaults for anything unset. Recognized inputs:
#   COMPOSITOR_RENDERER, COMPOSITOR_RENDERER_FALLBACK, COMPOSITOR_RENDERER_SYNC,
#   COMPOSITOR_HDR, COMPOSITOR_DEPTH, COMPOSITOR_VRR, COMPOSITOR_RENDER_NODE,
#   COMPOSITOR_DESKTOP_NAME, COMPOSITOR_LOG_LEVEL, Y5_VK_DIAG,
#   COMPOSITOR_CAPTURE_ENCODER, COMPOSITOR_WINDOW_CLIENT_SIZE_FALLBACK,
#   COMPOSITOR_WINDOW_SUBSURFACE_SHRINKS
compositor_env_json() {
    local renderer="${COMPOSITOR_RENDERER:-vulkan}"
    # KMS IN_FENCE by default, matching config.base::RENDERER_SYNC_DEFAULT. Set
    # COMPOSITOR_RENDERER_SYNC=sync for the synchronous device_wait_idle path.
    local renderer_sync="${COMPOSITOR_RENDERER_SYNC:-infence}"
    local render_node="${COMPOSITOR_RENDER_NODE:-/dev/dri/renderD128}"
    local desktop_name="${COMPOSITOR_DESKTOP_NAME:-Y5Compositor}"
    local log_level="${COMPOSITOR_LOG_LEVEL:-info,warn,error}"
    local vk_diag="${Y5_VK_DIAG:-}"
    local capture_encoder="${COMPOSITOR_CAPTURE_ENCODER:-nvenc}"
    local capture_codec="${COMPOSITOR_CAPTURE_CODEC:-av1}"
    local capture_quality="${COMPOSITOR_CAPTURE_QUALITY:-optimized}"
    local capture_refresh_rate_max="${COMPOSITOR_CAPTURE_REFRESH_RATE_MAX:-120}"
    local capture_background_encoder="${COMPOSITOR_CAPTURE_BACKGROUND_ENCODER:-ffmpeg}"
    local capture_variable_frame_rate
    capture_variable_frame_rate="$(_y5_bool "${COMPOSITOR_CAPTURE_VARIABLE_FRAME_RATE:-1}")"
    local capture_nvenc_allow_readback_fallback
    capture_nvenc_allow_readback_fallback="$(_y5_bool "${COMPOSITOR_CAPTURE_NVENC_ALLOW_READBACK_FALLBACK:-0}")"
    local fallback
    fallback="$(_y5_bool "${COMPOSITOR_RENDERER_FALLBACK:-}")"
    local hdr
    hdr="$(_y5_bool "${COMPOSITOR_HDR:-}")"
    local win_size_fallback
    win_size_fallback="$(_y5_bool "${COMPOSITOR_WINDOW_CLIENT_SIZE_FALLBACK:-}")"
    local win_subsurface_shrinks
    win_subsurface_shrinks="$(_y5_bool "${COMPOSITOR_WINDOW_SUBSURFACE_SHRINKS:-}")"

    # VRR is ON unless explicitly disabled.
    local vrr=true
    case "$(printf '%s' "${COMPOSITOR_VRR:-1}" | tr '[:upper:]' '[:lower:]')" in
        0 | off | false | no) vrr=false ;;
    esac

    # Depth: only "10" engages 10-bit; everything else is 8-bit.
    local depth=8
    [ "${COMPOSITOR_DEPTH:-}" = "10" ] && depth=10

    printf '{"renderer":"%s","renderer_fallback":%s,"renderer_sync":"%s","hdr":%s,"depth":%s,"vrr":%s,"render_node":"%s","desktop_name":"%s","log_level":"%s","vk_diag":"%s","capture_encoder":"%s","capture_codec":"%s","capture_quality":"%s","capture_refresh_rate_max":%s,"capture_background_encoder":"%s","capture_nvenc_allow_readback_fallback":%s,"capture_variable_frame_rate":%s,"window_client_size_fallback":%s,"window_subsurface_shrinks":%s}' \
        "$renderer" "$fallback" "$renderer_sync" "$hdr" "$depth" "$vrr" \
        "$render_node" "$desktop_name" "$log_level" "$vk_diag" "$capture_encoder" \
        "$capture_codec" "$capture_quality" "$capture_refresh_rate_max" "$capture_background_encoder" \
        "$capture_nvenc_allow_readback_fallback" "$capture_variable_frame_rate" \
        "$win_size_fallback" "$win_subsurface_shrinks"
}

# Resolve the settings-file path the compositor reads ($XDG_CONFIG_HOME wins, else
# $HOME/.config). Keep in lockstep with config.base::resolve_path().
compositor_settings_path() {
    printf '%s/y5.compositor/settings.json' "${XDG_CONFIG_HOME:-$HOME/.config}"
}

# Write the assembled settings JSON to the settings file (creating the directory).
compositor_write_settings() {
    local path
    path="$(compositor_settings_path)"
    mkdir -p "$(dirname "$path")"
    compositor_env_json > "$path"
    echo ">> wrote compositor settings $path" >&2
}

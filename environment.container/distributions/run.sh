#!/usr/bin/env bash
# (A) Open a shell on a target distro (nested under the host's Wayland session), with the winit
# y5_compositor compiled-on-that-distro and an `exec.sh` ready to launch it. You land in a shell
# so you can pre-check the distro (libs, GPU, `ldd /usr/local/bin/y5_compositor`, `vulkaninfo`,
# ...); type `exec.sh` to write settings + start the compositor.
#
# Usage: ./run.sh <distro>            (always the devloop image)
#
# Differs from ../run.sh on purpose:
#   * no --network flag (removed)
#   * the source is git-cloned into the image, not mounted/copied — so this runs the
#     binary that was actually COMPILED ON THAT DISTRO. Re-run image.sh after code changes.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$HERE/common.sh"

DISTRO="${1:?usage: run.sh <distro>  (have: $(distro_list | tr '\n' ' ')) }"
# Dev-loop entry point: it can only ever build the nested winit image, so it names
# that mode explicitly rather than defaulting to it. Release bundles come from
# `image.sh <distro> bundle`, never from here.
MODE=devloop

distro_validate "$DISTRO"

IMAGE="$(distro_image "$DISTRO" "$MODE")"
NAME="$(distro_container "$DISTRO")"

podman image exists "$IMAGE" || "$HERE/image.sh" "$DISTRO" "$MODE"

podman rm -f -t 0 "$NAME" 2>/dev/null || true

# --- GPU via the NVIDIA Container Toolkit (CDI) ----------------------------------------------
# The host GPU is passed as a CDI device produced by `nvidia-ctk cdi generate` (writes a spec to
# /etc/cdi/, which podman resolves). CDI injects the host's NVIDIA userspace, so the distro images
# only ship mesa + the Vulkan loader. Pick a specific GPU with Y5_CDI_DEVICE=nvidia.com/gpu=0.
CDI_DEVICE="${Y5_CDI_DEVICE:-nvidia.com/gpu=all}"
cdi_preflight() {
    # OK if nvidia-ctk lists the device, or a spec dir already references an nvidia.com/gpu device.
    if command -v nvidia-ctk >/dev/null 2>&1 && nvidia-ctk cdi list 2>/dev/null | grep -qF "$CDI_DEVICE"; then
        return 0
    fi
    grep -rqsF "nvidia.com/gpu" /etc/cdi /var/run/cdi 2>/dev/null && return 0
    echo "WARNING: NVIDIA CDI device '$CDI_DEVICE' not found — GPU passthrough will fail." >&2
    echo "  Generate the spec on the host:  sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml" >&2
    echo "  Then verify:                    nvidia-ctk cdi list" >&2
}
cdi_preflight

# Honor a host-set log level; default to the usual dev verbosity otherwise.
LOG_LEVEL="${COMPOSITOR_LOG_LEVEL:-info,warn,error,trace}"

echo ">> shell on $DISTRO (devloop), nested under $WAYLAND_DISPLAY." >&2
echo "   compositor: /usr/local/bin/y5_compositor — run 'exec.sh' to write settings + launch it." >&2
echo "   ('exit' or Ctrl-D leaves the shell and removes the container.)" >&2
# Mounts:
#   - the host Wayland socket (so the nested compositor can connect)
#   - exec.sh into PATH (launches the compositor on demand)
#   - the LIVE settings-writer over the image's copy, so settings fixes apply without a rebuild
podman run -it --rm \
    --name "$NAME" \
    --init \
    --env-file "$ENV_CONTAINER" \
    -e COMPOSITOR_LOG_LEVEL="$LOG_LEVEL" \
    --device "$CDI_DEVICE" \
    -v "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY:/tmp/wayland-host:ro" \
    -v "$HERE/exec.sh:/usr/local/bin/exec.sh:ro" \
    -v "$HERE/gpu-setup.sh:/usr/local/bin/gpu-setup.sh:ro" \
    -v "$REPO_ROOT/environment/compositor-env.sh:/working.directory/environment/compositor-env.sh:ro" \
    --security-opt label=disable \
    -w /working.directory \
    "$IMAGE" /bin/bash -c '/usr/local/bin/gpu-setup.sh || true; exec bash -i'

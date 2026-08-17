#!/usr/bin/env bash
# Run the udev (DRM/KMS) backend inside a nested virtio-gpu KVM VM.
#
# The compositor's udev backend needs a real KMS card + a seat + DRM master — none of which a
# container provides. This boots a lightweight VM (virtme-ng, an installed kernel + this rootfs)
# with a virtio-gpu device whose GL is accelerated by the host GPU via virglrenderer, starts a
# seat (seatd) inside, and runs the udev binary there. No compositor source is modified.
#
# Usage: ./run.udev.local.sh
# Always release-fast — build.sh has no profile argument.
#
# Prerequisites (host/sandbox — done once, as root, by whoever provisions the container):
#   1. Relaunch the sandbox with KVM exposed:   --device /dev/kvm   (keep /dev/dri/renderD129)
#   2. Install VM + guest-GL + seat tooling AND a bootable kernel:
#        dnf install -y virtme-ng qemu-system-x86-core virglrenderer \
#                       mesa-dri-drivers mesa-vulkan-drivers seatd \
#                       kernel-core kernel-modules-core
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RENDERNODE="${Y5_VIRGL_RENDERNODE:-/dev/dri/renderD129}"   # host GPU that backs virgl

# ── prerequisite checks ─────────────────────────────────────────────────────────────────
missing=0
{ [ -e /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; } || {
    echo "ERROR: /dev/kvm not present/RW — relaunch the sandbox with --device /dev/kvm" >&2; missing=1; }
for t in vng qemu-system-x86_64 seatd seatd-launch; do
    command -v "$t" >/dev/null || { echo "ERROR: missing tool '$t'" >&2; missing=1; }
done
KVER="$(ls -1 /lib/modules 2>/dev/null | sort -V | tail -1 || true)"
[ -n "$KVER" ] || { echo "ERROR: no kernel under /lib/modules — install kernel-core kernel-modules-core" >&2; missing=1; }
[ -e "$RENDERNODE" ] || { echo "ERROR: render node $RENDERNODE missing (set Y5_VIRGL_RENDERNODE)" >&2; missing=1; }
if [ "$missing" != 0 ]; then
    cat >&2 <<'EOF'

To enable, as root install:
  dnf install -y virtme-ng qemu-system-x86-core virglrenderer mesa-dri-drivers \
                 mesa-vulkan-drivers seatd kernel-core kernel-modules-core
and relaunch the container with: --device /dev/kvm
EOF
    exit 1
fi

# ── reap stale VMs from prior runs ───────────────────────────────────────────────────────────
# A killed vng can leave its qemu + helpers (virgl_render_server, virtiofsd) alive, each holding a
# host GPU context. Enough of them starve a new VM's venus/wgpu init, so the iced overlay can't
# create its context and disconnects ~0.4s in (looks like "the compositor closes immediately").
# Clear leftover y5 udev VMs before launching. Safe: pkill skips this script's own process, and
# the qemu match is scoped to our virtio-gpu-gl invocation.
if pgrep -f 'qemu-system-x86_64.*virtio-gpu-gl' >/dev/null 2>&1; then
    echo ">> reaping stale y5 udev VM(s) from previous runs ..." >&2
    pkill -f 'qemu-system-x86_64.*virtio-gpu-gl' 2>/dev/null || true
    pkill -x virgl_render_se 2>/dev/null || true
    pkill -x virtiofsd       2>/dev/null || true
    sleep 2
fi

# ── build the udev binary (host side; virtme mounts this rootfs, so the guest sees it) ──────
BIN="$("$HERE/../environment/build.sh" udev)"

# The compositor's settings JSON, computed host-side for the guest (written to the
# guest's settings file below): in the VM the only card is the virtio-gpu
# (/dev/dri/renderD128, the y5 default), so pin the render node to that regardless
# of the host's container.env value.
# shellcheck disable=SC1091
. "$HERE/../environment/compositor-env.sh"
GUEST_ENV_JSON="$(COMPOSITOR_RENDER_NODE=/dev/dri/renderD128 \
    COMPOSITOR_LOG_LEVEL="${COMPOSITOR_LOG_LEVEL:-info,warn,error,trace}" \
    compositor_env_json)"

# ── in-guest runner (executed as root inside the VM) ────────────────────────────────────────
GUEST="$(mktemp /tmp/y5-udev-guest.XXXX.sh)"
cat > "$GUEST" <<EOF
#!/bin/bash
export XDG_RUNTIME_DIR=/tmp
# The compositor reads its config from a settings file (computed host-side above).
_y5_cfg="\${XDG_CONFIG_HOME:-\${HOME:-/root}/.config}/y5.compositor"
mkdir -p "\$_y5_cfg"
printf '%s' '${GUEST_ENV_JSON}' > "\$_y5_cfg/settings.json"
export COMPOSITOR_LOG_LEVEL="${COMPOSITOR_LOG_LEVEL:-info,warn,error,trace}"
export COMPOSITOR_NESTED="${COMPOSITOR_NESTED:-1}"   # nested shortcut behavior (override from host env)
# guest-side GPU/Vulkan debug knobs (host-controlled; absent = off). VN_DEBUG drives the Mesa venus
# (guest virtio Vulkan) driver; MESA_* the guest GL/Vulkan stack. vng gives the guest a minimal env,
# so these must be injected here rather than inherited.
${VN_DEBUG:+export VN_DEBUG="$VN_DEBUG"}
${MESA_VK_ABORT_ON_DEVICE_LOSS:+export MESA_VK_ABORT_ON_DEVICE_LOSS="$MESA_VK_ABORT_ON_DEVICE_LOSS"}
${MESA_DEBUG:+export MESA_DEBUG="$MESA_DEBUG"}
${MESA_LOG:+export MESA_LOG="$MESA_LOG"}
${RUST_BACKTRACE:+export RUST_BACKTRACE="$RUST_BACKTRACE"}
modprobe virtio-gpu 2>/dev/null || true
udevadm trigger 2>/dev/null || true; sleep 0.5
echo "[guest] kernel: \$(uname -r)"
echo "[guest] /dev/dri:"; ls -l /dev/dri/ 2>&1 || echo "  (no DRM — virtio-gpu missing)"
# In the guest the only card is the virtio-gpu → /dev/dri/card0 + renderD128 (the y5 default).
echo "[guest] launching udev compositor under seatd ..."
exec seatd-launch -- "$BIN"
EOF
chmod +x "$GUEST"

# ── boot the VM with an accelerated virtio-gpu and run the guest script ──────────────────────
# qemu's egl-headless does virgl on the host render node via EGL/GBM — it needs the same NVIDIA
# EGL/GBM environment the compositor uses (GBM_BACKEND=nvidia-drm, EGL vendor ICDs, …).
set -a
# shellcheck disable=SC1091
. "$HERE/container.env"
set +a

# virtio-gpu-gl-pci needs a GL-capable qemu display backend for virgl. NVIDIA's raw render-node
# EGL (egl-headless) fails, but the Wayland EGL path works, so use gtk,gl=on (opens a window on
# the host's wayland-host session — you can watch the udev compositor render). Override with
# Y5_QEMU_DISPLAY (e.g. "egl-headless,rendernode=$RENDERNODE" or "sdl,gl=on").
QEMU_DISPLAY="${Y5_QEMU_DISPLAY:-gtk,gl=on}"
VM_MEM="${Y5_VM_MEMORY:-4G}"

# venus = accelerated guest Vulkan (host RTX 4090 via virglrenderer). Needs blob + a host-visible
# window (hostmem); virtme-ng's shareable memfd RAM (-numa node,memdev=mem,share=on) covers the
# shared-memory requirement, so only the device flags are needed. virgl (GLES) + venus (Vulkan)
# both ride the one virtio-gpu-gl device. Disable with Y5_VENUS=0 (falls back to llvmpipe/CPU).
gpu="virtio-gpu-gl-pci"
if [ "${Y5_VENUS:-1}" != 0 ]; then
    gpu="$gpu,blob=true,venus=true,hostmem=$VM_MEM,max_hostmem=$VM_MEM"
fi

echo ">> booting virtio-gpu VM (kernel $KVER, mem $VM_MEM, venus=${Y5_VENUS:-1}, display $QEMU_DISPLAY), launching udev ..." >&2
exec vng --run "$KVER" \
    --disable-microvm \
    --user root \
    --cpus 4 --memory "$VM_MEM" \
    --rwdir=/tmp \
    --qemu-opts="-device $gpu -display $QEMU_DISPLAY" \
    --exec "bash $GUEST"

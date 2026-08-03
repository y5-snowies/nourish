#!/usr/bin/env bash
# Populate a self-contained aarch64 sysroot from Fedora packages, so the compositor
# can be cross-compiled on an x86_64 host with nothing else present — no container,
# no access to the target device.
#
# Usage: ./sysroot.sh [releasever]        (default: 44 -> ./fedora44)
#
# Run this ONCE. The result is a directory of headers, .so files and .pc files that
# `build.sh` (next to this script) links against. Re-run it only to move to a new
# Fedora release, or after adding a package below.
#
# Env:
#   Y5_SYSROOT_DIR   where to populate (default: <this dir>/fedora<releasever>)
#   Y5_RPM_CACHE     rpm download cache (default: <this dir>/.rpm.cache — kept OUTSIDE the
#                    sysroot so that directory is nothing but headers/libs; reused across runs)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REL="${1:-44}"
SYSROOT="${Y5_SYSROOT_DIR:-$HERE/fedora$REL}"
CACHE="${Y5_RPM_CACHE:-$HERE/.rpm.cache}"

command -v dnf        >/dev/null 2>&1 || { echo "sysroot.sh: needs dnf (to fetch aarch64 rpms)" >&2; exit 1; }
command -v rpm2archive >/dev/null 2>&1 || { echo "sysroot.sh: needs rpm2archive (rpm package)" >&2; exit 1; }

# The link-time closure the compositor needs. Mirrors environment/install-deps.sh,
# minus everything that runs on the HOST at build time (clang, protoc, node): those
# stay x86_64 and come from the host system, not from this sysroot.
PKGS=(
    # libc / compiler runtime / kernel uapi headers
    glibc-devel libgcc libstdc++-devel kernel-headers
    # Wayland + smithay core
    wayland-devel wayland-protocols-devel
    libinput-devel libseat-devel libxkbcommon-devel pixman-devel
    libdrm-devel libdisplay-info-devel
    # graphics stack
    mesa-libgbm-devel mesa-libEGL-devel mesa-libGL-devel libglvnd-devel
    vulkan-loader-devel
    # capture / encode (compositor.y5 graphic.capture)
    ffmpeg-free-devel libva-devel
    # system integration
    systemd-devel dbus-devel pam-devel pulseaudio-libs-devel
    # xwayland-satellite / xcb
    libxcb-devel xcb-util-cursor-devel
)

mkdir -p "$SYSROOT" "$CACHE"

# --forcearch: fetch the aarch64 build of each package on an x86_64 host.
# --releasever: pin to the DEVICE's Fedora release — this is what fixes the glibc
#   ABI the binary is built against, so the result actually runs there.
# --alldeps: without it dnf skips dependencies that are "already installed", which
#   here means the HOST's x86_64 copies — the sysroot would come out full of holes.
echo ">> downloading Fedora $REL aarch64 packages (cache: $CACHE)" >&2
dnf download --forcearch=aarch64 --releasever="$REL" --resolve --alldeps \
    --destdir="$CACHE" "${PKGS[@]}" >&2

# --overwrite: packages legitimately share paths (and rpm ships 0555 files, which
#   tar cannot truncate in place), so without it a later package aborts the run.
# --keep-directory-symlink: preserves merged-/usr (lib64 -> usr/lib64) instead of
#   replacing the symlink with a real directory halfway through.
extract_one() {
    rpm2archive -n "$1" 2>/dev/null \
        | tar -x -C "$SYSROOT" --no-same-owner --overwrite --keep-directory-symlink 2>/dev/null
}

# ORDER MATTERS. The `filesystem` package owns /usr/lib64, /usr/lib, /usr/bin ...
# and ships them mode 0555, so every package extracted AFTER it fails to write its
# libraries there — the sysroot then comes out with all the headers and .pc files
# but almost no .so files, which only surfaces as a link error much later. So:
# extract the directory-owning packages first, make the tree writable, and keep them
# out of the main pass (re-extracting them would re-apply 0555 and break everything
# downstream again).
echo ">> extracting into $SYSROOT" >&2
skel=()
for rpm in "$CACHE"/filesystem-*.rpm "$CACHE"/setup-*.rpm "$CACHE"/basesystem-*.rpm; do
    [ -e "$rpm" ] || continue
    extract_one "$rpm" || true
    skel+=("$rpm")
done
chmod -R u+w "$SYSROOT"

failed=0
for rpm in "$CACHE"/*.rpm; do
    skip=
    for s in "${skel[@]}"; do [ "$rpm" = "$s" ] && skip=1 && break; done
    [ -n "$skip" ] && continue
    extract_one "$rpm" || { failed=$((failed + 1)); echo "   warn: $(basename "$rpm")" >&2; }
done
chmod -R u+w "$SYSROOT"
[ "$failed" = 0 ] || echo ">> note: $failed package(s) reported extraction warnings" >&2

# --- Prune ------------------------------------------------------------------
# The dependency closure drags in whole runtime packages (bash, coreutils, python).
# Nothing outside headers / libraries / pkg-config data can be linked against, and
# dropping it takes the sysroot from ~1.4 GB to a few hundred MB.
echo ">> pruning non-link-time files" >&2
rm -rf "$SYSROOT"/{bin,sbin,boot,dev,etc,home,proc,root,run,srv,sys,tmp,var,opt} \
       "$SYSROOT"/usr/{bin,sbin,libexec,src,games} \
       "$SYSROOT"/usr/lib/{python*,systemd,udev,dracut,firmware,modules,sysusers.d,tmpfiles.d,rpm} \
       "$SYSROOT"/usr/lib64/{python*,security,gconv} 2>/dev/null || true
# /usr/share: keep only pkgconfig (some .pc files live there) and the wayland
# protocol XML that wayland-scanner reads at build time.
if [ -d "$SYSROOT/usr/share" ]; then
    find "$SYSROOT/usr/share" -mindepth 1 -maxdepth 1 \
        ! -name pkgconfig ! -name 'wayland*' ! -name 'aclocal' -exec rm -rf {} + 2>/dev/null || true
fi

# --- Merged-/usr symlinks ---------------------------------------------------
# Fedora is merged-usr: /lib64 is a symlink to usr/lib64. Some .pc files and DT_NEEDED
# paths still say /lib64, so recreate the links (rpm extraction can leave real dirs).
for pair in "lib64:usr/lib64" "lib:usr/lib"; do
    link="$SYSROOT/${pair%%:*}"; tgt="${pair##*:}"
    if [ -d "$link" ] && [ ! -L "$link" ]; then
        cp -a "$link/." "$SYSROOT/$tgt/" 2>/dev/null || true
        rm -rf "$link"
    fi
    [ -e "$link" ] || ln -s "$tgt" "$link"
done

# --- Repoint absolute symlinks ----------------------------------------------
# A packaged rootfs is full of links like /usr/lib64/libEGL.so -> /usr/lib64/libEGL.so.1.
# Left absolute they resolve against the HOST's x86_64 filesystem, so the cross linker
# would either miss them or pick up a wrong-architecture library.
echo ">> repointing absolute symlinks into the sysroot" >&2
python3 - "$SYSROOT" <<'PY'
import os, sys
root = os.path.abspath(sys.argv[1])
fixed = 0
for dirpath, dirnames, filenames in os.walk(root):
    if os.path.islink(dirpath):
        continue
    for name in dirnames + filenames:
        p = os.path.join(dirpath, name)
        if not os.path.islink(p):
            continue
        target = os.readlink(p)
        if not target.startswith('/'):
            continue
        os.remove(p)
        os.symlink(os.path.relpath(os.path.join(root, target.lstrip('/')), dirpath), p)
        fixed += 1
print(f">> repointed {fixed} absolute symlinks", file=sys.stderr)
PY

# --- Shims ------------------------------------------------------------------
# A few libraries ship their runtime `.so.N` in one package and the linkable `.so`
# symlink in another that this sysroot has no other reason to carry — on Fedora,
# libgcc_s.so lives in `gcc` (a ~90 MB compiler we do not need; the cross gcc on the
# host supplies the driver). rustc's unwinder emits -lgcc_s unconditionally, and ld
# only accepts the bare `.so` name, so create the missing links here.
for lib in libgcc_s libstdc++; do
    so="$SYSROOT/usr/lib64/$lib.so"
    [ -e "$so" ] && continue
    real="$(ls -1 "$SYSROOT/usr/lib64/$lib.so."[0-9]* 2>/dev/null | head -n1)" || true
    [ -n "$real" ] && ln -sf "$(basename "$real")" "$so" && echo ">> shim: $lib.so -> $(basename "$real")" >&2
done

# --- Verify ------------------------------------------------------------------
# Link-critical files, checked explicitly: a sysroot can contain every header and
# every .pc file and still be unusable if the libraries themselves failed to land
# (see the two-pass note above). Fail loudly here rather than 20 minutes into a build.
echo ">> verifying link inputs:" >&2
lib64="$SYSROOT/usr/lib64"
broken=0
for f in crt1.o crti.o crtn.o libc.so libm.so libgcc_s.so \
         libwayland-server.so libwayland-client.so libinput.so libseat.so libudev.so \
         libEGL.so libGL.so libgbm.so libdrm.so libvulkan.so libxkbcommon.so \
         libpixman-1.so libdbus-1.so libpulse.so libdisplay-info.so libavformat.so; do
    if [ -e "$lib64/$f" ]; then printf '   ok      %s\n' "$f" >&2
    else printf '   MISSING %s\n' "$f" >&2; broken=1; fi
done
[ "$broken" = 0 ] || { echo ">> sysroot is INCOMPLETE — do not build against it" >&2; exit 1; }

# --- Report ------------------------------------------------------------------
echo ">> pkg-config coverage:" >&2
missing=0
for pc in wayland-server wayland-client libdrm gbm egl glesv2 gl vulkan libinput libseat \
          libudev xkbcommon pixman-1 dbus-1 libpulse libdisplay-info libavformat libavcodec libavutil libva; do
    if find "$SYSROOT" -name "$pc.pc" -print -quit 2>/dev/null | grep -q .; then
        printf '   ok      %s\n' "$pc" >&2
    else
        printf '   MISSING %s\n' "$pc" >&2; missing=1
    fi
done
[ "$missing" = 0 ] || echo ">> add the providing package to PKGS in this script and re-run" >&2

echo ">> sysroot ready: $(du -sh "$SYSROOT" | cut -f1) at $SYSROOT" >&2
echo "$SYSROOT"

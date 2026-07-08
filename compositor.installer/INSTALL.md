# Installing y5

The y5 compositor ships as **prebuilt release tarballs**. The Fedora 44 bundle is the
main one (the one-liner below); per-distro × per-arch bundles for **Debian 12/13,
Ubuntu 24.04/26.04 and Arch** are published as the `bundles-rolling` GitHub release. Because
the binaries are already compiled, installing pulls only the runtime shared libraries —
no Rust toolchain, no `-devel` headers.

The interactive installer is **distro-aware**: it detects your package manager
(`dnf` / `apt-get` / `pacman`) from `/etc/os-release` and installs that distro's runtime
package names. On **NixOS** the interactive installer does not apply — NixOS is declarative and non-FHS,
so its `sudo`-into-`/usr` session steps don't fit. Instead the bundle ships a native entry
point: unpack any glibc bundle (e.g. the Fedora one) and run **`./y5-install/nixos-setup.sh`**.
It prints the ready `programs.nix-ld` module (`nixos/configuration-y5.nix`, the runtime libs
as nixpkgs attributes) and how to add it; after `sudo nixos-rebuild switch`, nix-ld lets the
prebuilt binaries in `y5-install/binaries/` run. (The wayland-session / display-manager wiring
is still manual on NixOS — a full declarative flake/module is future work.)

## One command (any distro — auto-detects)

```bash
curl -fsSL https://nourish.snowies.com/install | bash
```

That runs the universal bootstrap ([`bootstrap.sh`](bootstrap.sh)): it detects your distro
and CPU arch, downloads the matching `package-<distro>-<arch>.tar.gz` from the multiarch
release, **verifies it against `SHA256SUMS` (mandatory — no skip)**, unpacks it, and launches
the interactive installer. On **NixOS** it instead fetches a glibc bundle and runs its
`nixos-setup.sh` (prints the nix-ld module/flake — installs nothing). Run it as **your normal
user, not with `sudo`** — the installer invokes `sudo` itself only for system steps, so your
config lands in `$HOME/.config` (it refuses to run as root). Safe to re-run.

Pin a specific release with `Y5_RELEASE_TAG` (e.g. `bundles-v1.4.1-rc.2`), force a target
with `Y5_DISTRO`/`Y5_ARCH`, or list what's available with `bootstrap.sh --list`.

### Fedora only (the classic path)

```bash
curl -fsSL https://nourish.snowies.com/release/latest/fedora44/package.tar.gz | tar -xz && y5-install/install.sh
```

This pulls the Fedora bundle directly (via [`get.sh`](get.sh) behind the shorter
`…/install-fedora` URL). It unpacks to `./y5-install/` and launches the same installer.

### Preview without changing anything

```bash
curl -fsSL https://nourish.snowies.com/release/latest/fedora44/package.tar.gz | tar -xz && y5-install/install.sh --dry-run
```

After install, log out and pick the **"Y5 Compositor"** session in your display manager.

## What the installer does

1. **Detects your GPU** (`lspci`). On NVIDIA it does not install a driver — it checks
   the bound kernel driver and warns if `nouveau` is in use or none is bound (see the
   NVIDIA note below).
2. **Installs the runtime packages** (see below) via the detected package manager
   (`dnf` / `apt-get` / `pacman`). The install is **strict**: if a package is unavailable,
   it aborts rather than continuing half-installed. Every default package is in that
   distro's base repos, so this only fails if your repos are broken. (Debian 12 also
   enables `bookworm-backports` for `libdisplay-info2`; NixOS prints a profile instead.)
3. Prompts the Y5 Compositor configuration (render node, color depth, VRR, …) and
   **seeds `~/.config/y5.compositor/settings.json`** from your answers. The session
   wrapper never rewrites it afterwards, so later edits (via `y5.compositor.settings`)
   stick.
4. Lays down the single **Y5 Compositor** session — its `/usr/bin` wrapper, systemd
   user service, wayland-session entry and xdg-desktop-portal config.
5. Optionally installs the developer tool window, the polkit agent, the MX gesture
   daemon and the PAM lock policy.

## Runtime libraries (what actually gets installed)

These are the **exact** runtime dependencies of the prebuilt `y5_compositor`
binary, derived from the binary itself — its ELF `NEEDED` entries (directly linked)
plus the sonames it `dlopen`s at runtime (Wayland, Vulkan, EGL):

The same soname set is installed under each distro's own package names (the installer
picks the right column from `/etc/os-release`):

| Shared library (how used) | Fedora (`dnf`) | Debian/Ubuntu (`apt`) | Arch (`pacman`) | NixOS (nixpkgs) |
| --- | --- | --- | --- | --- |
| `libpam.so.0` (linked) | `pam` | `libpam0g` | `pam` | `pam` |
| `libdbus-1.so.3` (linked) | `dbus-libs` | `libdbus-1-3` | `dbus` | `dbus` |
| `libpulse.so.0` (linked) | `pulseaudio-libs` | `libpulse0` | `libpulse` | `libpulseaudio` |
| `libudev.so.1` (linked) | `systemd-libs` | `libudev1` | `systemd-libs` | `systemd` |
| `libinput.so.10` (linked) | `libinput` | `libinput10` | `libinput` | `libinput` |
| `libseat.so.1` (linked) | `libseat` | `libseat1` | `seatd` | `seatd` |
| `libxkbcommon.so.0` (linked) | `libxkbcommon` | `libxkbcommon0` | `libxkbcommon` | `libxkbcommon` |
| `libpixman-1.so.0` (linked) | `pixman` | `libpixman-1-0` | `pixman` | `pixman` |
| `libgbm.so.1`, `libdrm.so.2` (linked) | `mesa-libgbm`, `libdrm` | `libgbm1`, `libdrm2` | `mesa`, `libdrm` | `mesa`, `libdrm` |
| `libdisplay-info.so.N` (linked) | `libdisplay-info` | `libdisplay-info{1,2,3}`¹ | `libdisplay-info` | `libdisplay-info` |
| `libav*.so` (linked, capture) | `libav*-free` | `ffmpeg`² | `ffmpeg` | `ffmpeg` |
| `libwayland-{client,server,egl}.so` (dlopen) | `libwayland-{client,server,egl}` | `libwayland-{client0,server0,egl1}` | `wayland` | `wayland` |
| `libvulkan.so.1` + driver (dlopen, renderer) | `vulkan-loader` + `mesa-vulkan-drivers` | `libvulkan1` + `mesa-vulkan-drivers` | `vulkan-icd-loader` + `vulkan-swrast`/vendor³ | `vulkan-loader` |
| `libEGL.so.1` / GLES (dlopen, fallback) | `libglvnd-egl`/`-gles` + `mesa-libEGL` | `libglvnd0`, `libegl1`, `libgles2`, `libegl-mesa0` | `libglvnd` (via `mesa`) | `libglvnd`, `libGL` |
| Xwayland (X11 clients) | `xorg-x11-server-Xwayland` | `xwayland` | `xorg-xwayland` | `xwayland` |

¹ Soversion tracks each release's `libdisplay-info-dev`: Debian 12/13 → `libdisplay-info2`
(Debian 12 from `bookworm-backports`, enabled automatically), Ubuntu 24.04 → `libdisplay-info1`,
Ubuntu 26.04 → `libdisplay-info3`. ² The `ffmpeg` package pulls the
exact soversion-suffixed `libav*` runtime for the release, so we don't name it. ³ Arch has
no generic `mesa-vulkan-drivers`; the installer adds the vendor ICD (`vulkan-radeon` /
`vulkan-intel`) for the detected GPU on top of the software `vulkan-swrast`.

On NixOS these attributes go into `programs.nix-ld.libraries` (the installer prints the
snippet) rather than being installed system-wide.

The generic Mesa Vulkan driver (for AMD/Intel) ships in the required `runtime` group, so
**Vulkan rendering works with no extra repos** on every distro.

> **Note for AMD users:** some have reported that Vulkan does not work on AMD. If
> you're on an AMD card, set `renderer` to `gles` when prompted.

**Hardware video acceleration (optional, opt-in):** the VA-API video driver
(`mesa-va-drivers-freeworld`) is one Fedora can't ship, so it lives in RPM Fusion. The
installer offers an explicit prompt — say yes and it enables RPM Fusion (free) for you,
then installs it; say no (the default) and it's never touched. This is only for VA-API
video decode/encode (e.g. faster capture), not for Vulkan rendering.

**NVIDIA:** the installer does **not** install the proprietary NVIDIA driver — the
`akmod-nvidia` stack needs a kernel-module build, a reboot, and Secure Boot signing,
so it's left to you. When an NVIDIA GPU is detected the installer instead checks the
bound kernel driver and prints a prominent warning if `nouveau` is in use (or no
driver is bound), with instructions to install the driver via RPM Fusion's
`akmod-nvidia` or NVIDIA's own `.run` installer, then reboot and re-run.

You do **not** need the build toolchain. The installer offers a `toolchain` group
(off by default) only for the rare case of compiling y5 on the target.

## Building & publishing the artifact

Maintainers build the hostable tarball on a machine with the toolchain:

```bash
compositor.installer/prepare.sh          # -> dist/package.tar.gz + dist/SHA256SUMS
```

Then upload both files to the release path the command above fetches:

```
https://nourish.snowies.com/release/latest/fedora44/package.tar.gz
https://nourish.snowies.com/release/latest/fedora44/SHA256SUMS
```

`get.sh` verifies the tarball against `SHA256SUMS` when it is published.

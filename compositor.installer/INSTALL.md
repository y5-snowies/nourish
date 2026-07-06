# Installing y5

The y5 compositor ships as a **prebuilt release tarball** for **Fedora 44**. Because
the binaries are already compiled, installing pulls only the runtime shared
libraries — no Rust toolchain, no `-devel` headers.

## One command

```bash
curl -fsSL https://nourish.snowies.com/release/latest/fedora44/package.tar.gz | tar -xz && y5-install/install.sh
```

That downloads the release, unpacks it to `./y5-install/`, and launches the
interactive installer. Run it as **your normal user, not with `sudo`** — it invokes
`sudo` itself only for the system-level steps, so your configuration lands in your
`$HOME/.config` (it refuses to run as root). It is safe to re-run.

If you host the bootstrap script ([`get.sh`](get.sh)) at a stable URL, the command
becomes even shorter:

```bash
curl -fsSL https://nourish.snowies.com/install | bash
```

### Preview without changing anything

```bash
curl -fsSL https://nourish.snowies.com/release/latest/fedora44/package.tar.gz | tar -xz && y5-install/install.sh --dry-run
```

After install, log out and pick the **"Y5 Compositor"** session in your display manager.

## What the installer does

1. **Detects your GPU** (`lspci`). On NVIDIA it does not install a driver — it checks
   the bound kernel driver and warns if `nouveau` is in use or none is bound (see the
   NVIDIA note below).
2. **Installs the runtime packages** (see below) via `dnf`. The install is **strict**:
   if a package is unavailable, it aborts rather than continuing half-installed. Every
   default package is in base Fedora repos, so this only fails if your repos are broken.
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

| Shared library | Fedora package | How the binary uses it |
| --- | --- | --- |
| `libpam.so.0` | `pam` | linked |
| `libdbus-1.so.3` | `dbus-libs` | linked |
| `libpulse.so.0` | `pulseaudio-libs` | linked |
| `libudev.so.1` | `systemd-libs` | linked |
| `libinput.so.10` | `libinput` | linked |
| `libseat.so.1` | `libseat` | linked |
| `libxkbcommon.so.0` | `libxkbcommon` | linked |
| `libpixman-1.so.0` | `pixman` | linked |
| `libgbm.so.1` | `mesa-libgbm` (→ `libdrm`) | linked |
| `libwayland-{client,server}.so.0` | `libwayland-client`, `libwayland-server` | dlopen |
| `libwayland-egl.so.1` | `libwayland-egl` | dlopen |
| `libvulkan.so.1` | `vulkan-loader` + driver | dlopen (default renderer) |
| `libEGL.so.1` / GLES | `libglvnd-egl`/`libglvnd-gles` + `mesa-libEGL` | dlopen (GLES fallback) |

The generic Mesa Vulkan driver (`mesa-vulkan-drivers`, for AMD/Intel) ships in the
required `runtime` group, so **Vulkan rendering works with no extra repos**;
`libdisplay-info` (EDID parsing) and `xorg-x11-server-Xwayland` (X11 clients) round
out the default set.

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

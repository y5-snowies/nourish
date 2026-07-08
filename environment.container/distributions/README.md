# distributions/ — test y5_compositor across Linux distros

Build and run the compositor on **different distributions** to check portability. Each distro is
a subdirectory with its own `Containerfile`, **named `<distro>-<version>`** (e.g. `fedora-43`,
`fedora-44`, `ubuntu-24.04`, `ubuntu-26.04`, `debian-11`, `debian-12`, `debian-13`, and `arch` —
rolling, so unversioned).
Base images are pinned (no `:latest`). The driver scripts
take that full name as the `<distro>` argument and the image tags carry the version too. Three
driver scripts at the top do the work:

| Script              | Purpose                                                                 |
| ------------------- | ----------------------------------------------------------------------- |
| `prepare-source.sh` | **Run in the sandbox** — materialize a self-contained clone (`.src`)     |
| `image.sh`          | Build the per-distro image (clone `.src` + compile the winit binary)     |
| `run.sh`            | **(A)** Open a shell on that distro; `exec.sh` launches the winit compositor |
| `build.sh`          | **(B)** Compile the binary on that distro and extract it to the host      |

```
distributions/
  common.sh           # shared helpers (repo root, distro discovery, image/container names)
  prepare-source.sh   # ./prepare-source.sh            (run in the sandbox; writes .src)
  image.sh            # ./image.sh <distro> [debug|release]
  run.sh              # ./run.sh   <distro> [debug|release]
  build.sh            # ./build.sh <distro> [debug|release] [out-dir]
  fedora-43/Containerfile
  fedora-44/Containerfile
  ubuntu-24.04/Containerfile
  ubuntu-26.04/Containerfile
  debian-11/Containerfile
  debian-12/Containerfile
  debian-13/Containerfile
  arch/Containerfile
  .src/               # self-contained source clone (gitignored)
  .cache/<distro>/    # per-distro cargo target cache — warm incremental rebuilds (gitignored)
  out/                # extracted binaries (gitignored)
```

## How the source gets in (clone, not copy) — two phases

Unlike `../Containerfile`, these images **do not `COPY` the live workspace** and do **not** use a
`--network` flag. Instead the prepared source tree is bind-mounted read-only at `/repo` during
the build (via `image.sh`'s `podman build -v <.src>:/repo:ro`) and each `Containerfile` clones it:

```dockerfile
RUN git clone /repo /working.directory
```

(The source is mounted with `-v` rather than the inline `--mount=type=bind,source=.` context
mount, which podman does not populate from the build context — that produced
`fatal: repository '/repo' does not exist`. The build context itself is just the Containerfile
dir; the source arrives through the `-v` mount.)

The catch is **linked git worktrees**: a worktree's `.git` is a *file* pointing at an external
gitdir (e.g. `/home/y5/nourish/.git`). Only a machine that has that gitdir can clone the
worktree. The **dev sandbox has it; the host does not.** So the flow is split:

1. **In the sandbox** (has the full git data): run `./prepare-source.sh`. It clones the repo into
   `distributions/.src` — a **normal, self-contained repo** (real `.git/`, no external link),
   containing only committed/tracked files (no `target/`, no untracked cruft).
2. **Anywhere** (sandbox *or* host): `image.sh`/`build.sh`/`run.sh` build using `.src` as the
   context. Since `.src` has no external worktree link, this works on the host even though the
   host can't resolve the original worktree's `.git`.

`image.sh` auto-runs step 1 if `.src` is missing **and** the git data is available — so in the
sandbox a bare `./image.sh fedora` just works. For the host, materialize `.src` in the sandbox
first (it lives under the worktree, so a shared filesystem carries it over; otherwise copy it).

- **Only committed changes are built.** Commit (or stash→commit), then re-run `prepare-source.sh`
  to pick up local edits.
- Override the location with `Y5_DIST_SRC=/path` (must be readable wherever you run `podman`).

## Usage

```bash
# (A) Open a shell on a distro (nested under your current Wayland session):
./run.sh fedora-43           # debug
./run.sh ubuntu-24.04 release
./run.sh debian-12
./run.sh arch

#   You land in a shell — pre-check the distro, then launch the compositor:
#     [container]$ ldd /usr/local/bin/y5_compositor   # check linkage
#     [container]$ vulkaninfo | head                  # check GPU/Vulkan
#     [container]$ exec.sh                             # write settings + start winit (Ctrl-C to quit)
#   Open a client inside the running session from another terminal (any Wayland app, e.g. foot):
#     podman exec -it -e WAYLAND_DISPLAY=wayland-1 y5-distro-fedora-43 foot

# (B) Compile + extract the binary for a distro (lands in ./out/<distro>/):
./build.sh ubuntu-24.04 release
./build.sh debian-11         # -> out/debian-11/y5_compositor

# Rebuild an image after committing source changes:
./image.sh arch release
```

`run.sh` reuses the GPU/session env from `../container.env` (NVIDIA EGL paths,
`COMPOSITOR_RENDER_NODE`, `WAYLAND_DISPLAY=wayland-host`) and passes the host GPU via a **CDI
device from the NVIDIA Container Toolkit** (`nvidia-ctk`). The NVIDIA userspace is injected by CDI,
so the distro images only ship mesa + the Vulkan loader. Prerequisite on the host (once):

```bash
sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml   # generate the CDI spec
nvidia-ctk cdi list                                          # verify nvidia.com/gpu=... devices
```

`run.sh` defaults to `--device nvidia.com/gpu=all` and preflights that the device is registered
(warning with the command above if not). Target a specific GPU with `Y5_CDI_DEVICE=nvidia.com/gpu=0`.
The compositor it launches (via `exec.sh`) is the binary that was **compiled on that distro** (the
image bakes it in), so re-run `image.sh` after changing code.

### NVIDIA gbm backend (`gpu-setup.sh`)

`container.env` sets `GBM_BACKEND=nvidia-drm`, so mesa needs `nvidia-drm_gbm.so` in the distro's
gbm dir. CDI injects the backend lib (`libnvidia-allocator.so`) but creates the symlink against the
**host's** gbm path — wrong for a Debian/Ubuntu image (`/usr/lib/x86_64-linux-gnu/gbm`). Without it
you get `MESA-LOADER: failed to open nvidia-drm` → mesa falls back to `DRM_IOCTL_MODE_CREATE_DUMB`
on a render node → `Permission denied` and a `CreateBo` panic. `gpu-setup.sh` (run by `exec.sh`
and at shell start, idempotent) links `nvidia-drm_gbm.so` → the injected allocator so allocations
take the NVIDIA path. It no-ops where the symlink already exists (e.g. a Fedora image on a Fedora
host). Check the render node it uses is your NVIDIA card: `cat /sys/class/drm/renderD12*/device/vendor`
(`0x10de` = NVIDIA); set `COMPOSITOR_RENDER_NODE=/dev/dri/renderDXXX` if needed.

`exec.sh` writes the required `settings.json` from the `COMPOSITOR_*` env (via the repo's
`environment/compositor-env.sh`, which has every required field). `run.sh` mounts that settings
writer **live** over the image's copy, so a settings-writer fix takes effect immediately — no
image rebuild needed for that part (only a code/binary change needs a rebuild).

## Incremental rebuilds (per-distro target cache)

When the clone changes, the image's `git clone` layer changes and its `cargo build` re-runs — but
it does **not** start from scratch. `image.sh` mounts a **per-distro** cargo target dir from the
host (`.cache/<distro>/`) at `/y5-target` (the Containerfiles set `Y5_TARGET_DIR` to match), so the
compiled artifacts persist between builds. The (large, vendored) dependency graph stays compiled;
cargo only rebuilds what actually changed. Each distro gets its own cache dir — they have
incompatible ABIs/toolchains and must not share. Override the base with `Y5_DIST_CACHE=/path`;
`podman` never commits this dir into the image, so images stay small.

> Caveat: `git clone` gives every file a fresh mtime, so cargo re-checks (and may recompile) the
> local workspace crates even when their content is unchanged — but the dependency graph (the bulk
> of the build) is reused. To also skip unchanged local crates, restore commit mtimes after the
> clone (e.g. `git-restore-mtime`); ask and I can add that to the Containerfiles.

Clear a cache with `rm -rf .cache/<distro>` (or all of `.cache/`).

## Adding a distro

Create `distributions/<distro>-<version>/Containerfile` following one of the existing ones —
install that distro's equivalents of the build deps (rust/cargo, clang/libclang, pkg-config,
protobuf, the Wayland/smithay devel libs, mesa + Vulkan loader, ffmpeg, dbus + pulse), then the
same `git clone /repo` + `environment/build.sh winit ${PROFILE}` steps. Use the `<distro>-<version>`
naming so the image tag carries the version. The driver scripts discover it automatically — no
edits needed (`./image.sh <distro>-<version>` just works). Package names differ across releases;
adjust as needed (e.g. `debian-11` may lack a few newer devel libs).

## Verifying the installer's runtime package names (`verify-packages.sh`)

The installer (`y5-install`) installs **runtime** package names per distro (see
`compositor.installer` — `dnf`/`apt`/`pacman`/nix-ld). Those names differ from the `-dev` build
deps in these Containerfiles and have real cross-release hazards (the `t64` renames on
trixie/noble, soversion suffixes like `libdisplay-info2`, Arch's vendor-split Vulkan ICDs). The
**multiarch CI never runs the installer**, so a wrong name wouldn't fail CI — and a full CI run is
heavy (native builds incl. a Tauri app).

`verify-packages.sh` closes that gap **without building anything**: it reuses each Containerfile's
pinned `FROM` **base image**, refreshes only the package index, and checks that every name the
installer would install actually resolves in that distro's repos. Seconds per distro, just
`podman pull` + metadata.

```bash
./verify-packages.sh                       # fedora-44 debian-12/13 ubuntu-24.04/26.04 arch nixos
./verify-packages.sh debian-13 arch        # a subset
./verify-packages.sh nixos                 # just the NixOS nixpkgs attrs
```

The name list comes straight from the installer (`y5-install --emit-packages=<mgr>[:<rel>]`), so
it validates the exact table the real install uses. Run this **before** pushing to the `candidate`
branch; a non-zero exit lists the missing package(s) per distro. Needs `podman` (same prerequisite
as the other scripts here).

**NixOS** is checked differently: it has no Containerfile (nix-ld runs a glibc bundle, nothing is
built for it), so `verify-packages.sh nixos` runs one evaluate-only `nix eval` (no build) inside
the `nixos/nix` image and reports any nixpkgs attribute the installer's nix-ld profile names that
doesn't exist in `nixpkgs-unstable`. It fetches nixpkgs (a one-off download) and needs network.
Override the image with `Y5_NIX_IMAGE=`.

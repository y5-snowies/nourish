# environment/ — build, run & deploy the compositor

Everything needed to build the `y5_compositor` binary, run it on the host, and deploy a
release build. The **containerized** dev loop (a Fedora dev image under nested Wayland) lives
in the sibling **`environment.container/`** folder. Scripts resolve the repo root themselves
(`$REPO_ROOT`), so they work from any cwd and don't hard-code directory depth — moving these
folders won't break them.

## Layout

```
environment/                  # host build / run / release (no container)
  build.sh            # compile y5_compositor          [winit|udev] [debug|release]
  run-host.sh         # run on the HOST, no container  [winit|udev] [debug|release] [--it] [--env=FILE]
  build-release.sh    # host udev release build + install/deploy  <dev|system|remote>
  install-deps.sh     # install host build deps on Fedora (for bare-metal builds)
  check.sh            # workspace lint/conformance gate
  compositor-env.sh   # turn COMPOSITOR_* knobs into the settings.json the binary reads

environment.container/        # the containerized dev loop under nested Wayland
  Containerfile       # Fedora-based dev image (stable cargo/rust, Wayland + GPU stack)
  container.env       # env vars for the containerized run (NVIDIA path)
  realmachine.env     # env vars for a bare-metal run (Intel/VA-API path)
  image.sh            # build the dev container image
  run.sh              # run in the container           [winit|udev] [debug|release]
  run.local.sh        # run on the host using container.env (NVIDIA)  [winit|udev] [debug|release]
  run.udev.local.sh   # run the udev backend in a nested QEMU/seat    [debug|release]
  entrypoint.sh       # in-container build+run (invoked by run.sh, not by hand)
  launch.sh           # launch a client app into the running container  [app]
  stop.sh             # stop/remove the dev container
  distributions/      # build/run on different distros (Fedora/Ubuntu/Debian/Arch, version-
                      #   suffixed); clones the local repo instead of COPYing — see its README.md
```

The host scripts (`build.sh`, `build-release.sh`, `install-deps.sh`) are also what CI invokes;
`environment.container/` is developer-only and unused by CI. The container scripts delegate
compilation and config back to `../environment/build.sh` and `../environment/compositor-env.sh`,
so the backend/profile logic still lives in exactly one place.

## Compositor configuration (one file)

The compositor reads **all** of its own configuration from a single JSON file,
`~/.config/y5.compositor/settings.json` (override with `--config-file=<path>`) — a
JSON object whose fields are all **required** (`renderer`, `renderer_fallback`,
`renderer_sync`, `hdr`, `depth`, `vrr`, `render_node`, `scanout_node`, `desktop_name`, `log_level`,
`vk_diag`, `capture_encoder`, `window_client_size_fallback`,
`window_subsurface_shrinks`). It is parsed once at the top of `main()` and the
process **panics** if the file is missing or any field is absent — there are no
defaults and no optionals. No compositor config is read from the environment.

The run scripts write this file for you: `compositor-env.sh` turns the familiar
individual knobs (`COMPOSITOR_RENDERER`, `COMPOSITOR_DEPTH`, …, and `Y5_VK_DIAG`)
into the JSON and `compositor_write_settings` drops it at the settings path,
applying defaults for anything unset — so `COMPOSITOR_RENDERER=gles ./run-host.sh`
still works. In production the installer's session wrapper writes the same file
before launch. To author it by hand interactively, use the `y5.compositor.settings`
tool (`compositor.installer/component/settings-editor`).

## Backends (winit vs udev)

The binary is built against the loader's entry crate
(`compositor.loader/.../main.execute/execute`). The backend is chosen at **compile
time** by the `backend-winit` (default) / `backend-native` cargo features —
`main.rs` switches on `#[cfg(feature = "backend-native")]`:

| Backend | Cargo                                              | Use                                            |
| ------- | -------------------------------------------------- | ---------------------------------------------- |
| `winit` | (default, `backend-winit`)                         | nested — runs inside an existing Wayland/X session |
| `udev`  | `--no-default-features --features backend-native`  | DRM/KMS — runs on real hardware / a TTY        |

Backend and profile (`debug`/`release`) are independent axes — any combination is
valid. `build.sh` is the single place that knows this mapping; everything else
delegates to it.

## Common workflows

```bash
# --- in environment/ (host) ---

# Just compile (prints the built binary's path). Pick backend + profile:
./build.sh                 # winit debug   (default)
./build.sh udev            # udev  debug
./build.sh udev release    # udev  release
./build.sh winit release   # winit release

# Dev loop on the HOST (no container) — builds via build.sh then execs the binary.
./run-host.sh                       # winit debug, nested in your current session
./run-host.sh --it                  # ...but prompt for every env var first
COMPOSITOR_RENDERER=gles ./run-host.sh   # force GLES (Vulkan is the default)
./run-host.sh udev release --env=../environment.container/container.env   # bare-metal NVIDIA udev

# Release build on the host + install/deploy (always udev release):
./build-release.sh dev     # -> /usr/bin/y5.compositor.dev   (sudo cp)
./build-release.sh system  # -> /usr/bin/y5.compositor       (sudo mv)
./build-release.sh remote  # -> y5@yrd.local:/home/y5/compositor (scp)

# Bare-metal host build deps (Fedora):
./install-deps.sh

# --- in environment.container/ (containerized dev loop) ---

# One-time: build the dev image.
./image.sh

# Dev loop: build + run inside the container. Ctrl-C to quit.
./run.sh                 # winit debug, nested under the host Wayland session
./run.sh udev release    # udev release

# Open a client inside the running compositor (defaults to alacritty):
./launch.sh              # alacritty
./launch.sh chrome       # google-chrome-stable on Wayland

# Stop the container:
./stop.sh
```

The container name and image tag are both `y5-compositor-smithay-dev`. `run.sh` mounts
every top-level `compositor*` workspace plus `vendor/` automatically, so adding or
renaming a workspace needs no edit here.

## Build speed

Build accelerator, applied to both host and container builds:

| Accelerator | Where configured | What it does |
| ----------- | ---------------- | ------------ |
| **`line-tables-only` dev debug info** | `build.sh` (debug profile) | Keeps backtrace line numbers but cuts the debug binary ~3.4× (1.5 GB → ~440 MB) and speeds linking. |

(The **mold** linker was removed — it hung at link time in some environments. Builds
now use the default system linker.)

Install the host tooling with `./install-deps.sh` (pulls `clang`). Inside the
container, `run.sh` mounts `.cargo/config.toml` and a persistent cargo target dir.

> Cargo does **not** merge `RUSTFLAGS` with `.cargo/config.toml` rustflags — a set
> `RUSTFLAGS` replaces them and drops the `-A warnings` flag. The scripts deliberately
> don't export it; don't add it back.

> sccache was evaluated and removed: on a many-core host with a warm `target/`, the
> dependency graph it caches is already cheap to rebuild in parallel, so it gave no
> wall-time win for the local/container workflow. If a CI runner or a cache shared
> across machines is ever added, re-enable it there by exporting
> `RUSTC_WRAPPER=sccache` in that environment — no repo changes needed.

> Removed in the consolidation (kept in git history if ever needed): the old `arc/`
> directory of dead "anvil" container experiments, fully-commented-out script stubs, and
> a stale hard-coded deploy path (`deploy.locally.nolink.sh`). Their live behavior is
> covered by the scripts above.

# environment/ — build, run & deploy the compositor

Everything needed to build the `y5_compositor` binary, run it on the host, and deploy a
release build. The **containerized** dev loop (a Fedora dev image under nested Wayland) lives
in the sibling **`environment.container/`** folder. Scripts resolve the repo root themselves
(`$REPO_ROOT`), so they work from any cwd and don't hard-code directory depth — moving these
folders won't break them.

## Layout

```
environment/                  # host build / run / release (no container)
  build.sh            # compile y5_compositor          [winit|udev] [--deploy[=NAME]]
  build-optimized.sh  # ...but fat-LTO release         [winit|udev] [--deploy[=NAME]]
  run-host.sh         # run on the HOST, no container  [winit|udev] [--it] [--env=FILE]
  check.sh            # does it compile? lint gate + cargo check   [workspace-dir]
  install-deps.sh     # install host build deps on Fedora (for bare-metal builds)
  compositor-env.sh   # turn COMPOSITOR_* knobs into the settings.json the binary reads

document/shader-examples/     # the example shader bundles, and their installer
  install-shaders.sh  # copy the bundles beside it into <data>/background/shader  [--dry-run] [--prune] [BUNDLE ...]

environment.container/        # the containerized dev loop under nested Wayland
  Containerfile       # Fedora-based dev image (stable cargo/rust, Wayland + GPU stack)
  container.env       # env vars for the containerized run (NVIDIA path)
  realmachine.env     # env vars for a bare-metal run (Intel/VA-API path)
  image.sh            # build the dev container image
  run.sh              # run in the container           [winit|udev]
  run.local.sh        # run on the host using container.env (NVIDIA)  [winit|udev]
  run.udev.local.sh   # run the udev backend in a nested QEMU/seat
  entrypoint.sh       # in-container build+run (invoked by run.sh, not by hand)
  launch.sh           # launch a client app into the running container  [app]
  stop.sh             # stop/remove the dev container
  distributions/      # build/run on different distros (Fedora/Ubuntu/Debian/Arch, version-
                      #   suffixed); clones the local repo instead of COPYing — see its README.md
```

The host scripts (`build-optimized.sh`, `install-deps.sh`) are also what CI invokes;
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

The binary is built against the loader's entry crate — `build.sh` finds it by grepping
for the `y5_compositor` `[[bin]]` name, so it survives renames. The backend is chosen at **compile
time** by the `backend-winit` (default) / `backend-native` cargo features —
`main.rs` switches on `#[cfg(feature = "backend-native")]`:

| Backend | Cargo                                              | Use                                            |
| ------- | -------------------------------------------------- | ---------------------------------------------- |
| `winit` | (default, `backend-winit`)                         | nested — runs inside an existing Wayland/X session |
| `udev`  | `--no-default-features --features backend-native`  | DRM/KMS — runs on real hardware / a TTY        |

The backend is the only axis: there is no profile argument. See "Profiles" below.
`build.sh` is the single place that knows this mapping; everything else delegates to it.

## Profiles: there is one

Local work is **always `release-fast`** — release optimizations without LTO. It is not
selectable: `build.sh`, `run-host.sh` and `check.sh` take no profile argument and
reject one if given.

`debug` is gone everywhere. Its dependency tree cost ~4x the release-fast one
(measured in one worktree: 14 GB vs 3.6 GB) for a compositor nobody single-steps, and
being the default it is what every casual build produced. `release-fast` inherits
`debug = "line-tables-only"`, so backtraces still carry line numbers.

Fat-LTO `release` is **`build-optimized.sh`** and nothing else. Its final link is
serial and takes 10-20 minutes; it is for release tags and published artifacts, not
for finding out whether the tree compiles. That question is `check.sh`.

`[profile.release-fast]` is declared in **every** workspace root, because cargo only
honours `[profile.*]` at the root it is invoked from.

## One shared target dir

`.cargo/config.toml` pins `[build] target-dir` to the loader workspace's `target/` —
the same tree `build.sh` writes into — for the whole repo.

This matters because the repo is 30 independent workspaces, so cargo's default (a
`target/` per workspace root) means a full multi-GB dependency tree **per root** the
moment anything runs cargo outside `build.sh`. The path is relative to the config
file, not the cwd, so it resolves identically from every workspace and survives git
worktrees.

Command line and environment still win over it, which is how the exceptions keep
their own trees: the cross compiler (`cross.compile/build.sh` -> `~/.cache/y5-cross/target`),
the containers (`Y5_TARGET_DIR=/y5-target`) and the coverage runner
(`cover/lib/cover.sh`). Their artifacts are not interchangeable with the host's.

## Common workflows

```bash
# --- in environment/ (host) ---

# Does it compile? (lint gate + cargo check, one shared target dir)
./check.sh                          # every workspace root
./check.sh ../compositor.orchestration   # just one

# Just compile (prints the built binary's path). Backend is the only choice:
./build.sh                 # winit  (default)
./build.sh udev            # udev

# Dev loop on the HOST (no container) — builds via build.sh then execs the binary.
../document/shader-examples/install-shaders.sh             # refresh every example bundle in the data dir
../document/shader-examples/install-shaders.sh --dry-run   # ...or just show what has drifted
../document/shader-examples/install-shaders.sh tb-crt-spin # ...or one bundle
./run-host.sh                       # winit, nested in your current session
./run-host.sh --it                  # ...but prompt for every env var first
COMPOSITOR_RENDERER=gles ./run-host.sh   # force GLES (Vulkan is the default)
./run-host.sh udev --env=../environment.container/container.env   # bare-metal NVIDIA udev

# Build + install. Copies (never moves — a move forces a full relink next build)
# and re-applies cap_sys_nice, which the copy drops:
./build.sh udev --deploy                        # -> /usr/bin/y5.compositor
./build.sh udev --deploy=y5.compositor.exp      # -> /usr/bin/y5.compositor.exp

# The shipped artifact — fat LTO, 10-20 min. Release tags only:
./build-optimized.sh udev --deploy

# Bare-metal host build deps (Fedora):
./install-deps.sh

# --- in environment.container/ (containerized dev loop) ---

# One-time: build the dev image.
./image.sh

# Dev loop: build + run inside the container. Ctrl-C to quit.
./run.sh                 # winit, nested under the host Wayland session
./run.sh udev            # udev

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

The two things that actually make builds cheap here are covered above: **one shared
target dir**, and **release-fast instead of debug** (which is also ~4x less disk).

(The **mold** linker was removed — it hung at link time in some environments. Builds
now use the default system linker.)

Install the host tooling with `./install-deps.sh` (pulls `clang`). Inside the
container, `run.sh` mounts `.cargo/config.toml` and a persistent cargo target dir.

> Cargo does **not** merge `RUSTFLAGS` with `.cargo/config.toml` rustflags — a set
> `RUSTFLAGS` replaces them and drops both `-A warnings` and `-C target-cpu=x86-64-v3`.
> The scripts deliberately don't export it; don't add it back.

> That `target-cpu=x86-64-v3` (Haswell 2013+, AVX2/FMA/BMI2) applies to every profile
> built from this tree, and the binaries **SIGILL on pre-v3 CPUs**. Narrow or remove
> the `[target.'cfg(target_arch = "x86_64")']` block when producing bundles for
> generic hardware. aarch64 builds are unaffected.

> sccache was evaluated and removed: on a many-core host with a warm `target/`, the
> dependency graph it caches is already cheap to rebuild in parallel, so it gave no
> wall-time win for the local/container workflow. If a CI runner or a cache shared
> across machines is ever added, re-enable it there by exporting
> `RUSTC_WRAPPER=sccache` in that environment — no repo changes needed.

> Removed in the consolidation (kept in git history if ever needed): the old `arc/`
> directory of dead "anvil" container experiments, fully-commented-out script stubs, and
> a stale hard-coded deploy path (`deploy.locally.nolink.sh`). Their live behavior is
> covered by the scripts above.

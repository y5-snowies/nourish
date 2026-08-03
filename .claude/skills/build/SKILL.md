---
name: build
description: Build / compile / verify the y5_compositor binary. Use whenever you need to build the compositor, do a release build, install/deploy it, or confirm the whole tree still compiles. ALWAYS prefer the environment/ scripts over raw `cargo build` — the scripts share ONE target dir (disk-optimized); per-workspace `cargo build` scatters multi-GB target/ dirs across the tree.
---

# build

How to build the y5 compositor without bloating disk.

## Use the scripts, NOT raw `cargo build`

The repo is many independent top-level `compositor*` workspaces. Running
`cargo build` inside each one creates a **separate `target/` dir per workspace**
— each can be multiple GB, and shared deps recompile per workspace. This fills
the disk fast.

The `environment/` scripts route every build through **one shared target dir**
(the loader workspace's `target/`, or `Y5_TARGET_DIR`) and run the
workspace-lint gate first. Always prefer them.

## Build + install (the usual command)

```bash
environment/build-release.sh system    # udev release build → sudo mv → /usr/bin/y5.compositor
```

`build-release.sh <dev|system>` always builds the **udev backend in
release** (installs target real hardware), then:
- `dev`    → `sudo cp` to `/usr/bin/y5.compositor.dev`
- `system` → `sudo mv` to `/usr/bin/y5.compositor`

## Compile only (no install), pick backend + profile

```bash
environment/build.sh [winit|udev|native] [debug|release]   # default: winit debug
```

Prints the built binary's path as the only stdout line (logs go to stderr).
`winit` = nested backend (runs inside an existing Wayland/X session);
`udev`/`native` = DRM/KMS on real hardware/TTY. Debug builds use
line-tables-only debug info (smaller, faster links).

## Just checking it compiles

`environment/build.sh udev release` (or `winit debug` for speed) is the
whole-tree check — it discovers the entry crate and uses the shared target dir.
A scoped `cargo build -p <crate>` inside a single workspace is fine for a quick
type-check of one crate you're editing, but do **not** `cargo build` whole
workspaces one-by-one to "verify everything" — use the script.

## Notes

- Do **not** set `RUSTFLAGS` — it wipes `.cargo/config.toml` (warnings config +
  mold linker). The scripts already handle flags.
- `build.sh` runs `workspace.lint.js` first and fails the build on violations.
- Override the target dir with `Y5_TARGET_DIR=...` if you need it elsewhere.
- See `environment/README.md` → "Build speed" for the full rationale.

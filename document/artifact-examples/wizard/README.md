# wizard — the reference isolated artifact

An animated "electricity" border decomposed into four perimeter bars around every
window, following it as it moves/resizes and as the camera pans/zooms. **This is the
canonical (and only) home of the wizard** — the compositor tree carries host machinery
only. It is a fully isolated project: no workspace, no y5 path deps, just `abi_stable`
from crates.io plus the copied boundary (`abi.rs`).

## Layout

```
Cargo.toml         standalone cdylib workspace; deps: abi_stable + ./abi
abi/               VENDORED contract crate — canonical package name
                   `compositor_artifact_plugin_abi_base` @ 0.0.1, verbatim copy of
                   the in-tree crate. The name/version must match: abi_stable's
                   root-module verification checks the DEFINING PACKAGE's identity,
                   so a same-shaped copy under another package name is refused.
                   Replace with a registry dep once the contract is published.
lib.rs             pub mod wizard;
wizard.rs          the effect + the `#[export_root_module]` entry
y5.manifest.json   identity/version/kind/compat/entry (EXTENSIONS.md §5)
pack.sh            cargo build --release + zip → wizard.y5
```

## Build, pack, install, run

```bash
./pack.sh
mkdir -p ~/.local/share/y5/artifact && cp wizard.y5 ~/.local/share/y5/artifact/
environment/run-host.sh winit release
# (COMPOSITOR_ARTIFACT_PLUGINS=<dir> overrides the default artifact dir)
```

The loader validates the manifest at import (kind / `y5_api` / `abi_stable` line /
arch — mismatch = clean logged refusal), extracts `payload/plugin.so`, `dlopen`s it,
and injects it as a `System` in the main world. Without a `.y5` installed, no artifact
runs — the effect appearing at all is proof the plugin loaded.

## What to hack

- `BAND`, the flicker in `Wizard::draw`, or replace `perimeter_bars` entirely — any
  quads you return are composited at the world 3D band and follow the windows.
- Keep `abi.rs` byte-compatible with the host and `compat.y5_api` at `"1"`. If the
  host bumps its boundary, imports refuse cleanly until you rebuild against the new
  contract.

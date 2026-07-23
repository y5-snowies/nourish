# Artifact examples

Example **artifact extensions** — completely isolated projects that build outside the
y5 tree and are packaged as importable **`.y5`** files (see `document/EXTENSIONS.md`
and `document/EXTENSION-RUNTIMES.md`). This mirrors `shader-examples/`: each example is
a folder that stands alone.

An artifact here is a **1c native plugin**: a `cdylib` speaking only the stable-ABI
boundary (`abi_stable` `#[repr(C)]` types + one `#[sabi_trait]`). It has **no
y5/smithay dependency** — the host projects window geometry to physical and hands it
in; the plugin returns quads to composite. Because only the boundary is ABI-stable,
the plugin may be built with a different toolchain / dependency set than the
compositor and still load.

| Example | What it shows |
|---|---|
| `wizard/` | The reference artifact — and the wizard's **only** home (the compositor tree carries host machinery, no effect code): an animated "electricity" border decomposed into perimeter bars around every window. Standalone cargo project → `cargo build` → `pack.sh` → `wizard.y5`. |

## Using an example

```bash
cd document/artifact-examples/wizard
./pack.sh                                   # cargo build --release + zip → wizard.y5
mkdir -p ~/.local/share/y5/artifact && cp wizard.y5 ~/.local/share/y5/artifact/
environment/run-host.sh winit release       # or environment.container/run.sh debug
# (COMPOSITOR_ARTIFACT_PLUGINS=<dir> overrides the default artifact dir)
```

At startup the loader scans the dir, validates each manifest (kind / `y5_api` /
`abi_stable` line / arch — a mismatch is a clean logged refusal), extracts the
payload, `dlopen`s it, and injects it as a world `System`.

## The boundary contract

Each example VENDORS the contract crate (`abi/` — canonical package name
`compositor_artifact_plugin_abi_base`, verbatim): abi_stable's root-module check
compares the defining package's identity, so a plain file copy inside the plugin
package is refused at load. Vendoring the same-named crate is
sound **by construction**: identical definitions built against the same `abi_stable`
0.11 line produce identical layouts (that is the stable-ABI premise, and it is
runtime-verified — see EXTENSIONS.md §8). When the boundary is published as its
own crate (e.g. `y5-artifact-abi`), examples should depend on it instead of copying.
The contract version is `compat.y5_api` in the manifest; the host refuses a mismatch
at import time.

# EXTENSIONS.md — y5 native artifact extensions (the stable-ABI path)

Status: **implemented, runtime-verified, and exercised live in the compositor**
(2026-07-23). This is the single document for the **1c native path**: artifacts as
dynamically loaded `.so` plugins behind an `abi_stable` boundary, packaged as `.y5`
files, loaded at startup, participating as world `System`s. Non-native runtimes
(external-process bus, WASM / Lua / V8 scripting) are future work and live in
**`document/EXTENSION-RUNTIMES.md`**.

An **artifact** is, at runtime, a `System` participating in a world: it contributes
drawables (2D quads and host-drawn 3D meshes today; input claims and channels are
contract growth, §9), follows world state (windows, camera), and is loaded from a
`.y5` package rather than compiled in. The compositor tree carries **host machinery
only** — no effect code; effects are out-of-tree projects (see the wizard example,
§7).

---

## 1. Architecture map (all implemented)

### The quad primitive (lives in the ARTIFACT expansion, not orchestration)
- **`compositor_artifact_draw_quad_base`** (`artifact.draw/draw.quad/quad.base`) —
  `ArtifactQuad`: a System-pushable quad. Content is `ArtifactContent::Solid`
  (renderer-agnostic, zero GPU allocation) or `ArtifactContent::Dmabuf` (zero-copy
  import; the texture stretches to the quad's rect — any stretch, no implicit
  zoom/natural-size coupling). Placement is DECLARED, not pre-projected: an
  `ArtifactSpace` (`World` = world-logical f64, pans/zooms; `Screen` = physical px,
  fixed) + an f64 rect in that space. Orchestration consumes it the same way it
  consumes the expansion-owned `ParallaxBackground`.
- **`orchestration.draw/draw.node`** — `DrawNode::Artifact(ProjectedArtifact)`;
  `lower()` emits a solid element or imports the dmabuf into the projected rect;
  `ElementMeta` follows the quad's space (World quads count as world content for
  e.g. the Vulkan AA restriction, Screen quads as screen).
- **`orchestration.draw/draw.scene/scene.frame`** — `prepare()` demuxes the active
  world's single type-erased `FramePlan` by node type (`demux_world_nodes` —
  parallax + artifact quads, each keeping its declared band); `scene()` projects
  World quads through the active camera and plans everything at its band.
  Projection happens exactly once, here — contributors and plugins never project.

### Artifact workspace (`compositor.expansion/compositor.artifact`)
crates.io deps at the root: `abi_stable 0.11`, `libloading 0.8`, `zip 2`,
`serde_json 1` (user decision: no vendoring — "no need to fork if no need to patch").

```
artifact.plugin/
  plugin.abi/abi.base     THE CURRENT CONTRACT (v2) — and nothing else. §2
  plugin.v1/v1.abi        frozen v1 types (never edited again)             §3
  plugin.v1/v1.adapter    AdapterV1: v1 loading (raw symbol) + bridge      §3
  plugin.host/host.base   proxy PluginSystem — speaks ONLY the current
                          contract; routes by version                      §2,§3
  plugin.pack/pack.base   .y5 format: Manifest, write/read, compat gate    §5
artifact.draw/
  draw.quad/quad.base     ArtifactQuad + ProjectedArtifact (the primitive) §1
```

The tree carries **capability only** — zero content/demo code. Rendering decisions
(what, where, with which engine) live in plugins.

### Loader integration (`kernel.loader/.../execute.base/main.rs`)
`load_artifact_plugins()` scans **`$XDG_DATA_HOME/y5/artifact`** (default
`~/.local/share/y5/artifact`; `COMPOSITOR_ARTIFACT_PLUGINS` overrides) for `*.y5`,
validates + extracts (`/tmp/y5-artifact-plugins`), loads via the version router, and
injects each as a `System` into the main spatial world. Failures are logged and
skipped — never fatal; a missing default dir is silent (= nothing installed).

---

## 2. The contract (v2, current)

Only the boundary is ABI-stable — the plugin's internals (any Rust, any deps) never
cross it. At the boundary: `#[repr(C)]`/`StableAbi` types, `extern "C"` fn-pointers,
`R*` containers. Never `repr(Rust)`, `dyn Trait`, or std collections by value.

```rust
// plugin.abi/abi.base (contract y5_api = "2")
#[repr(C)] pub struct AbiRect { x, y, w, h: i32 }              // WORLD-logical units
#[repr(C)] pub struct AbiQuad { rect: AbiRect, color: [f32;4], band: u16 }
#[repr(C)] pub struct FrameCtx { windows: RVec<AbiRect>, dt: f32 }
#[repr(C)] pub struct AbiDmabufQuad {                           // plugin-RENDERED pixels
    id, commit: u64, fd: i32,                                  // fd plugin-owned; host dups
    width, height: i32, fourcc: u32, modifier: u64, stride, offset: u32,
    screen: u8, x, y, w, h: f64, band: u16,                    // placement (any stretch)
}

#[sabi_trait]
pub trait ArtifactPlugin {
    #[sabi(last_prefix_field)]                 // ← additive growth marker
    fn draw(&mut self, ctx: &FrameCtx) -> RVec<AbiQuad>;
    // SUFFIX (v2-additive; host probes absence): the ONE pixel primitive —
    // plugin-rendered dmabufs, any engine. screen_w/h let screen quads self-anchor.
    fn draw_dmabuf(&mut self, screen_w: f64, screen_h: f64) -> RVec<AbiDmabufQuad>;
}

#[sabi(kind(Prefix))]                          // ← add-only root module
pub struct ArtifactMod {
    #[sabi(last_prefix_field)]
    pub new: extern "C" fn() -> ArtifactPluginBox,
}
impl RootModule for ArtifactMod_Ref { /* BASE_NAME "artifact_plugin", version = pkg 0.0.x */ }
```

- The plugin exports the root module with **`#[export_root_module]`**; the host loads
  with `ArtifactMod_Ref::load_from_file` — abi_stable **recursively layout-verifies
  the whole boundary at load**. A mismatch is a clean `LibraryError`, never UB.
- **All coordinates crossing the boundary are WORLD-LOGICAL.** The proxy
  `PluginSystem` reads window rects via the `Platform` hatch and hands them across
  as-is; returned quads are wrapped as World-space `ArtifactQuad`s at each quad's
  **band**; the frame driver projects to physical (§1). The plugin is pure effect
  geometry with zero y5/smithay types and never projects — quad sizes therefore
  scale with camera zoom, as world content should. (`dt` is proxy-self-timed:
  `FrameTick.delta` is always zero, the pitfall `CameraSystem` documents.)
- **`band`** is the compositing position, plugin-chosen per quad per frame
  (clamped under `POINTER`). The band table (`world.frame`):

  ```
  BACKGROUND 0 · LAYER_BACKGROUND 10 · LAYER_BOTTOM 20 · WORLD_3D 100 ·
  ICED_WORLD 200 · CAPTURE_DIM 300 · CANVAS 400 (windows) ·
  401/402 RESERVED (floating panes) · CANVAS_ABOVE 410 · LAYER_TOP 450 ·
  LAYER_OVERLAY 480 · ICED_SCREEN 500 · POINTER 700
  ```

  The space is deliberately sparse — new bands slot between named ones. A future
  **`attach`** property will give a drawable a z *relative to another
  drawable/window* (content-band `DrawOrder` interleaving: "active window's z + ε");
  plain bands remain the mechanism for unattached artifacts.

**Boundary hazards (host discipline):** never let a panic unwind across the boundary
(`catch_unwind` → error/abort at every entry); memory is freed by its allocating side
(`R*` types carry their deallocator). Do not wait for language-level fixes (crABI
etc.) — the crate-level bridge is the production path.

---

## 3. Versioning & compatibility (the settled model)

**Additive-first.** Everything expressible additively goes AFTER a
`last_prefix_field` marker (trait methods, root-module entries): old plugins keep
loading; absent suffixes read as absent. No version bump, no adapter. By-value
structs (`AbiQuad`, `FrameCtx`) are **frozen at birth** — new data arrives as new
types through new suffix methods, never by editing an existing struct.

**Breaking changes mint a version layer.** When a break is unavoidable:
1. The outgoing contract is archived as its own frozen layer (`plugin.vN/vN.abi`) —
   never edited again.
2. Its **adapter** (`plugin.vN/vN.adapter`) is written *at break time*, bridging
   vN → current. The version layer **fully encapsulates** vN: its types, its loading
   mechanism, its `.so` keepalive. Nothing outside `plugin.vN/` may reference vN;
   the host proxy speaks only the current contract.
3. `HOST_Y5_API` bumps; the old version joins `SUPPORTED_Y5_APIS`.

**The adapter IS the capability decision** — there is no capability schema. If a
bridge is semantically acceptable (possibly with authored defaults — v1 quads have no
`band`, so AdapterV1 defaults them to `WORLD_3D`, logged once), the adapter exists
and the plugin works. If a break is too severe to bridge, the adapter simply isn't
written, and the version falls out of the supported set.

**Failure is binary, with two gates:**
- **Gate 1 — import** (`pack::check_compat`, before any code is mapped): manifest
  `y5_api` ∈ `SUPPORTED_Y5_APIS` (= versions with a complete adapter chain, known at
  compile time), `kind == native-stable`, `abi_stable` line matches, arch matches.
  Outside the set → logged refusal.
- **Gate 2 — load** (`PluginSystem::load`): version-routed; the current path runs
  abi_stable's recursive layout verification (and its package-version check — a
  plugin's crate version must track the contract's `0.0.x` line). Failure → logged
  error, library unloaded. The compositor is never affected.

**History:** v1 = raw `artifact_plugin_new` symbol, band-less quads (frozen in
`plugin.v1`). v2 = root module + layout verification + `band` (current).

---

## 4. Drawing rules

- **Zero-copy dmabuf is mandatory.** No CPU pixel path exists anywhere: content
  either never crosses a boundary (host-drawn — the host renders into its own
  dmabuf-backed textures) or crosses as a dmabuf fd (plugin-drawn, in-process by
  handle on the shared device; format/modifier must import without conversion, with
  explicit sync). `ArtifactContent::Solid` sidesteps pixels entirely.
- **Plugin quads** composite at their chosen band (§2) and are recomputed each frame
  (fresh element ids — animated content wants full repaint, mirrors `select.box`).
- **Host-drawn 3D** (§6) is parked; when re-wired via the bus it should carry a
  per-instance band like quads do (`CANVAS_ABOVE` is the named above-content band).

---

## 5. The `.y5` package (implemented)

A ZIP whose manifest is readable without extracting the payload:

```
wizard.y5
├── y5.manifest.json
└── payload/plugin.so
```

```json
{
  "schema": 1,
  "id": "dev.y5.examples.wizard",      // stable reverse-DNS identity
  "name": "Wizard (example artifact)",
  "version": "0.1.0",                  // semver of the extension itself
  "kind": "native-stable",
  "compat": {
    "y5_api": "2",                     // contract version → adapter-chain routing
    "abi_stable": "0.11",              // shared bridge line (the real cross-toolchain contract)
    "target_triple": "x86_64-unknown-linux-gnu"
  },
  "entry": "artifact_plugin_new"       // informational for v2 (root module is found by name)
}
```

Import pipeline: read manifest → **gate 1** (§3) → extract `payload/plugin.so` to
the cache → **gate 2** → wrap in the version-adapter chain → inject as a `System`.

Planned extensions to the format (not yet implemented): bundled `assets/` declared in
the manifest (the mesh asset currently installs beside the `.y5`, §6), a `y5-pack`
CLI that **stamps** compat fingerprints from the actual build instead of trusting the
authored manifest, `target`/instancing hints, and a detached signature.

---

## 6. Plugin 3D = plugin-rendered dmabuf (the dmabuf-lean decision)

Plugins that want 3D (or any custom pixels) **render themselves** — any engine —
into dmabufs they allocate, and hand only fds across `draw_dmabuf` (§2). The host
never gains a content vocabulary for it: fd → dup → smithay `Dmabuf` (cached per
plugin quad id, damage via `commit`) → the same `ArtifactQuad` primitive as solid
quads, with the declared space/band. Heavyweight plugins are accepted for now; a
LIGHTWEIGHT host-drawn convenience (host renders on the plugin's behalf) arrives
later as a bus/channel capability (§10) — never as trait methods.

Knowledge worth keeping from the (deleted) host-drawn experiments:
- The host's embedded bevy runs full `DefaultPlugins` (AssetPlugin + GltfPlugin
  present); glTF **scene** spawning via the fork's reflection-based `WorldAssetRoot`
  panics on unregistered component types — load **mesh primitives** directly
  (`GltfAssetLabel::Primitive`) instead.
- A `System` cannot hold `gles` (Platform hatch, `update`) and `&mut BG_THREE`
  (`buffer`) simultaneously — any future host-drawn path needs a rim/bus-side
  reconciler, like every existing 3D feature (lock, picker).

---

## 7. Authoring an artifact (the isolated example)

`document/artifact-examples/wizard/` is the wizard's **only** home and the template
artifacts follow: no y5 workspace; crates.io `abi_stable` + the contract crate
**vendored under its canonical package name** in `./abi` (abi_stable's root-module
verification checks the *defining package's* identity — a copied file inside the
plugin package is refused; the verbatim same-name/same-version crate passes, and
becomes a registry dep once published). The example **packages its own engine**: it
renders the spinning hat with bevy into a self-allocated gbm dmabuf (borrowing the
repo's vendored bevy/wgpu fork by path for the import helper — out-of-tree
artifacts ship their own engine + import plumbing) and returns two `AbiDmabufQuad`s
sharing that buffer: world (0,0) and screen top-right.

```bash
cd document/artifact-examples/wizard
./pack.sh                                   # cargo build --release + zip → wizard.y5
mkdir -p ~/.local/share/y5/artifact && cp wizard.y5 ~/.local/share/y5/artifact/
cp assets/wizard.gltf ~/.local/share/y5/artifact/   # enables the 3D mesh hook (§6)
environment/run-host.sh winit release
```

Vendoring the contract crate verbatim is sound by construction — identical
definitions in a same-named/same-versioned package produce identical layouts — and
gate 2 catches any drift at load. Keep the vendored crate byte-identical (name,
`0.0.x` version, contents); swap it for the registry dep once published.

---

## 8. Verification record

1. **Compile**: whole tree (`y5_compositor` release) + all artifact crates, 0 errors;
   `workspace.lint` 783 crates / 30 roots, 0 failures.
2. **ABI boundary (runtime)**: an independently-built loader (its own `abi_stable`
   compile — the faithful cross-build test) `dlopen`ed the plugin and `draw()`
   returned exactly the expected perimeter bars.
3. **`.y5` round-trip (runtime)**: pack → manifest read → compat gate → extract →
   load → drive. All OK.
4. **Live in the compositor**: `run-host.sh winit release` — plugin imported at
   startup (`artifact plugin loaded … (y5_api 2)`), electric border drawn at band
   410 above windows, spinning glTF wizard hat anchored left of the active window,
   both tracking windows and camera pan/zoom. Confirmed on screen.
5. **Not yet exercised at runtime**: the v1 adapter path (compile-verified only — no
   live v1 fixture since the example moved to v2; see §9).

---

## 9. Gaps / next steps

1. **`attach` z-order property** — drawables anchored to another drawable/window's z
   via the content-band `DrawOrder` authority (design agreed; bands are the interim).
2. **v1 CI fixture harness** — archive a v1 copy of the example source; CI builds,
   packs, and loads every archived version through the adapter chain each build,
   making "every version still operates" a checked invariant.
3. **Contract growth over suffix methods** — governed by the **dmabuf-lean
   decision**: the trait stays minimal; the ONE pixel primitive beyond solid quads
   is **plugin-drawn dmabuf** (fd + format/modifier + sync as suffix growth;
   `ArtifactContent::Dmabuf` already exists host-side). Content vocabularies —
   e.g. host-drawn mesh spawn/anchor — are deliberately NOT trait methods; they
   arrive as bus/channel capabilities (**§10**). Bus participation + channels RW
   are the big items.
4. **Textured electricity** — flipbook sprites via the dmabuf path instead of solid
   bars.
5. **`y5-pack` CLI** with build-stamped fingerprints (`write_y5` exists as a lib fn).
6. **Unload / hot-reload** — currently load-at-startup, process-lifetime by design.
7. **Active-window narrowing** for the border (currently every mapped window; the
   mesh already follows the active window via selection/keyboard focus).

---

## 10. The synchronous bus (design — the migration extensions plug into)

**Scope: this is NOT just the input bus.** It is a single, registration-based,
SYNCHRONOUS interception bus that successive compositor mechanisms migrate onto —
the existing world input bus is merely its first instance — up to and including the
**wire-protocol implementations** themselves. The end state: a bus participant
(built-in system or loaded artifact, same rules) can

- **listen** to any migrated event — a client's `xdg_toplevel.set_title`, decoration
  requests, lifecycle events, "all the other protocols";
- **swallow** an event (`Consume`) — the default implementation never runs, and the
  participant may implement the behaviour differently (e.g. take over title
  handling entirely);
- **annotate** an event — attach typed metadata that travels with it down the chain;
  later participants AND the default handler *adjust behaviour if they support the
  metadata* and ignore it otherwise (e.g. tag a title change with rendering hints).

### 10.1 The pattern

Participants register at world build: an **event-class selector** (input class,
wire interface/request — e.g. `xdg_toplevel.set_title` — lifecycle class, gate
class) + a **priority layer** (the existing `InputLayer` bands generalize:
`OVERLAY / SCREEN / WORLD / FALLBACK`, higher sees the event first; within a layer,
registration order). At each migrated dispatch point the chain runs synchronously —
protocol requests must resolve *now*, on the dispatch path — and each participant
returns a disposition:

```
Pass                        observe only
Consume                     swallow; default implementation skipped; participant owns it
Pass + annotations          metadata attached for downstream / the default handler
```

Synchronous is viable precisely because participants are in-process: a built-in
system is a direct call, a 1c artifact is one vtable hop across the boundary.
(Tier-2 external peers can never be synchronous — they participate via the
declarative-claim + lease model in EXTENSION-RUNTIMES.md §3, which is the async
projection of this same bus.)

### 10.2 Why the migration must come first: ordering honesty

Today the input bus runs BEFORE the entire legacy rim chain (the `should_forward`
gates: overlay shortcut table → dark gate → launcher → canvas shortcuts → exclusive
grab). Layer numbers therefore only order *bus members among themselves* — a
participant registered at `FALLBACK` still preempts every legacy gate, which is the
opposite of what the layer promises. **Layer semantics are only honest for
participants that are ON the bus.** Letting extensions register is therefore what
forces the migration: the remaining gates (and then the smithay dispatch delegates
for the wire protocols) become ordinary registered participants, the legacy chain
shrinks to "deliver to the focused client", and one ordering rule governs everyone.

### 10.3 Channels RW (rides the same machinery)

The per-world `ChannelRouter` already gives systems typed subscribe
(`builder.receive`) + emit (`cx.channels.send`). For artifacts, both cross the
contract as suffix growth:

- **Read** — the plugin declares subscriptions; the host proxy subscribes and
  delivers each message as a **curated `#[repr(C)]` mirror event** through an
  `on_event` suffix method. Curated mirrors, not generic serialization: each exposed
  channel gets a frozen boundary struct, exactly like `FrameCtx` (which is already a
  hand-rolled instance of this pattern).
- **Write** — the plugin returns emissions; the proxy validates the channel and
  forwards under its own identity. The host stays the authority on what an artifact
  may emit.

### 10.4 Phasing

1. **Bus core** — generalize the existing `InputBus` shape into the event envelope +
   disposition + metadata table (host-internal, no contract change).
2. **Gate migration** — the legacy input gates become registered participants
   (§10.2), continuing the existing rim→System "phase 3" direction.
3. **Wire-protocol migration** — thread the smithay dispatch delegates through the
   bus, interface by interface (title/app-id first: highest value, lowest risk),
   giving listen / swallow / annotate over protocol requests.
4. **Extension participation** — expose registration, `on_event`, dispositions, and
   annotations through the contract as **suffix methods** — v2-additive, existing
   plugins keep loading untouched (§3). Channels RW (§10.3) lands the same way.

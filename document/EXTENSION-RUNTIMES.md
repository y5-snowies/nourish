# EXTENSION-RUNTIMES.md — non-native extension runtimes (design)

Status: **design only.** The implemented native path (1c, `abi_stable` `.so`) lives in
`document/EXTENSIONS.md` and is the current focus. This document holds the designs for
every OTHER delivery mechanism — the external-process bus and the in-process scripting
runtimes (WASM, Lua, V8/JS) — so they stay specified without diluting the native doc.

The unifying principle carries over from the native path: **every mechanism crosses
the same kind of protocol — events in, commands + a render/texture handle out; never
live framework objects** (`iced::Element`, `bevy::App`). The 1c contract is the first
concrete instance; these runtimes are alternative *transports* for the same shape of
contract, and the `.y5` package is the shared distribution unit (`kind` selects the
runtime).

---

## 1. The delivery-mechanism space

Three axes: **locality** (in-process vs separate process), **boundary** (native /
VM / wire), **trust** (full memory access vs sandboxed).

| # | Mechanism | Locality | Boundary | Trust | Any lang | Own pixels | Compat |
|---|---|---|---|---|---|---|---|
| 1a | native `.so`, same toolchain | in-proc | `extern "Rust"` | trusted | ✗ | full | ✗ lockstep |
| **1c** | **native `.so`, stable ABI** — IMPLEMENTED, see EXTENSIONS.md | in-proc | `#[repr(C)]` | trusted | ✗ (Rust) | full | ✓ |
| 2 | external-process native bus | ext proc | wire | sandbox (OS) | ✓ | dmabuf | ✓ |
| 3 | in-process scripting VM | in-proc | VM | sandbox (VM) | ✓ (wasm) | host-drawn | ✓ |

Tiers are not "fast vs slow" — they are **trusted vs sandboxed**. 1a/1c share the
address space (a stable ABI fixes *compat*, not *trust*). Tier 2 has the best crash
isolation (separate process + seccomp/portals). Tier 3's permission model is enforced
by construction (strongest in wasm). Industry split matches: native stable-ABI for
trusted plugins (Tremor), WASM/WIT for untrusted (Zed, Zellij, Lapce), IPC for
robustness (Nushell).

Rejected / out of scope, recorded so boundaries stay explicit:
- **`dylib` + shared-std** (Bevy `dynamic_linking` style) — dev-only, more fragile
  than 1a; not a distribution mechanism.
- **Plain Wayland client** (layer-shell / toplevel) — presents surfaces but gets no
  world citizenship (no world-space camera, no input-bus ordering, no draw-order
  authority, no channels). The "do nothing new" floor tier 2 exists to beat.
- **gRPC remote** (`compositor.remote`) — stays as-is for coarse pub/sub only; the
  tier-2 bus is a separate, more native integration ("Remote stays remote").

---

## 2. The shared kernel (future generalization)

All runtimes target the same three pieces; the 1c implementation realizes them
narrowly today (proxy `PluginSystem`, `Manifest`, the v2 contract):

- **`PluginRegistry`** — constructs a participant from a validated manifest + a
  resolved transport and injects it into the target world (or rim). Supports N
  instances of one extension, each with independent state.
- **`HostCtx`** — the single capability surface. Everything an extension can do flows
  through it; if it is not on this object, no runtime can reach it. Planned surface:
  lifecycle (mirroring the `System` trait), drawables (host-drawn commands or
  plugin-drawn dmabuf), input claims (`InputLayer` + `InputFlow::Consume`), channel
  subscriptions, world-focus reads (accessors only — the WORLD-ID lint).
- **Layer/level declaration** — draw band, content-band tier, input layer, and
  world-vs-rim placement.

### Zero-copy rules per runtime
The compositor-wide constraint (no CPU pixel path, ever) lands differently per tier:
- **Host-drawn** (all tiers): trivially compliant — pixels never cross a boundary.
- **Plugin-drawn, tier 2**: dmabuf fd via `SCM_RIGHTS` (socket transport) or
  `zwp_linux_dmabuf` (Wayland transport), with format/modifier negotiated for
  conversion-free import and explicit sync (fences). Unimportable format = load-time
  rejection, never a CPU fallback.
- **Tier 3**: plugin-drawn is *impossible* (no GPU context in a sandbox) —
  scripting extensions are host-drawn only, by rule.

---

## 3. Tier 2 — the external-process bus

Any process participates as a world citizen over a native bus (distinct from the gRPC
remote). The hard constraint: the `System` hot path is synchronous, so an external
peer can never be on it directly.

**The pattern: every peer is fronted by an in-process proxy `System`** holding
authoritative local state, deciding everything synchronously from that state, and
talking to the peer only asynchronously.

- **Input — declarative claims + leases.** The peer registers combos/regions up
  front; the proxy returns `Consume` synchronously from local state and
  async-notifies the peer, whose reaction (e.g. open its launcher) is async. Claims
  are leases with deadlines: a dead peer's `Super+N` claim expires and falls back to
  the built-in handler. No synchronous IPC on the input path, ever.
- **Drawables — two sub-modes.** Host-drawn command protocol, or client-drawn dmabuf
  (the peer renders in its own GPU context — any engine — and submits fds; the
  compositor already imports dmabufs for iced/bevy, so this is "a Wayland client
  with world citizenship").
- **Lifecycle/channels** — forwarded async, best-effort, rate-limited/coalesced; the
  proxy bridges the world's `ChannelRouter` both directions.
- **Resilience (non-negotiable):** a broker task owns all sockets off-thread with
  bounded queues; heartbeat + deadline watchdog (degrade: freeze drawables, release
  leases, stop forwarding; socket EOF: evict); latest-wins backpressure. A
  misbehaving peer degrades gracefully — it can never stall the frame loop.
- **Transport decision:** custom Wayland protocol extension (most native — reuses
  smithay dispatch + dmabuf machinery; couples to the connection) vs dedicated Unix
  socket (decoupled; reimplements buffer plumbing). Wayland-protocol favored for the
  dmabuf negotiation it inherits.

Cost profile: best isolation and language freedom; highest build complexity; ≥1 frame
data-plane latency (fine for drawables; input decisions stay local).

---

## 4. Tier 3 — in-process scripting runtimes

Sandboxed by the language boundary: the VM only sees the host functions we register.
Withhold network/fs host-functions and the extension physically cannot reach them —
permissions are *enforced by construction* here (unlike the trusted native tiers,
where nothing is enforceable in-process).

| Engine | Langs | Weight | Speed | Sandbox character |
|---|---|---|---|---|
| **WASM (`wasmtime`)** | any that compiles to wasm | moderate | near-native + marshalling | capability by DEFAULT (imports-only, WASI grants) — strongest |
| **Lua (`mlua`)** | Lua | tiny | fast | opt-OUT (don't load `io`/`os`) |
| **JS/TS (`deno_core`)** | JS/TS | heavy (V8, tens of MB) | fastest (JIT) | op-surface only — `deno_core` has NO ambient ops (not the full Deno runtime) |
| **JS (`rquickjs`)** | JS | ~1 MB | slower (no JIT) | op-surface only |
| **JS (`boa`)** | JS | small, pure Rust | slowest | op-surface only (immature) |

Notes:
- **TypeScript routes:** real TS does not compile to wasm directly (type erasure →
  dynamic JS needing GC + engine). Options: AssemblyScript (TS-*like* dialect →
  lean wasm), Javy (QuickJS-in-wasm — real TS via transpile, bundles an engine), or
  first-class TS on `deno_core` outside wasm.
- **Lua-in-wasm** is possible (interpreter compiles to wasm) but normally pointless —
  embed `mlua` natively unless the hard wasm memory boundary is specifically wanted.
- **Drawables are host-drawn only** (declarative view/scene commands; the host owns
  the real iced/bevy objects) — a sandboxed script can't hand over framework objects
  or hold a dmabuf. Same marshalling shape as the tier-2 command protocol.
- The engine choice is not exclusive — multiple runtimes can coexist, each a `.y5`
  `kind` (`wasm` / `lua` / `js`) routed by the importer.

### Capabilities (sandboxed tiers only)
The native path deliberately has **no capability schema** (the adapter/trust model —
see EXTENSIONS.md §3). Sandboxed runtimes are where declared capabilities belong,
because here they are *enforceable*: the manifest requests (`filesystem`, `network`,
`gpu_scene`, …), policy grants, and the runtime exposes exactly the granted host
functions. A future manifest `capabilities` block applies to these kinds only.

---

## 5. Sequencing

These runtimes come after the native path's remaining gaps (EXTENSIONS.md §9):
1. **Tier 3 wasm (`wasmtime`)** first among these — strongest sandbox, any-language,
   and the `HostCtx` command surface it forces is the same one tier 2 needs.
2. **Tier 2 external bus** — broker, protocol, lease/deadline/backpressure machinery.
3. Lua / JS engines as demand warrants (each is "another transport for `HostCtx`").

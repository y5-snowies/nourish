# GPU topology: split render/scanout (PRIME) and the multi-GPU hardware zoo

y5's native backend historically assumed **one DRM device that both renders and
scans out**. That holds for a normal desktop GPU and breaks the moment the device
that *renders* differs from the device that *owns the display*. This document is
the map: what "split render/scanout" means, what has landed for the single-scanout
case (Jetson Orin / "the nano"), and the **deferred** plan for the general
multi-GPU zoo — including how CRTC management actually binds an output to a device.

Read `document/BRIDGE_PARALLAX.md` / `document/GLES_WGPU.md` first for the existing
dmabuf ↔ wgpu bridge: the render→scanout handoff here is the *same* mechanism
applied to a *second* device boundary.

## Vocabulary

- **Render node** — the GPU the compositor composites on (wgpu is pinned to it).
  A DRM *render* node (`/dev/dri/renderD*`) has no KMS.
- **Scanout device / card** — the KMS-capable DRM *card* (`/dev/dri/card*`) that
  owns CRTCs + connectors and can put a framebuffer on a wire.
- **CRTC** — the scanout engine inside a card that reads a framebuffer and drives a
  connector's timing. **A CRTC belongs to exactly one card.** There is no
  cross-card CRTC.
- **Connector** — a physical port (HDMI/DP/eDP). Enumerated by, and wired to, the
  CRTCs of **one** card.
- **Route** (`gpu.topology/topology.route`) — `None` when render card == scanout
  card (desktop, zero-copy is trivial), else `DmabufCopy { render, scanout }` = a
  cross-device handoff (which at execution is *import* or *blit* — see below).

## The two axes that decide everything

1. **Node split** — is the display a different DRM device than the render GPU?
   (Desktop: no. Orin, PRIME laptop, monitor-on-second-GPU: yes.)
2. **Memory topology** — do render and scanout share physical memory?
   - **Unified (UMA / SoC, e.g. Orin):** one physical GPU + one DRAM shared with
     the display controller. A render dmabuf already lives where the display can
     read it → cross-device handoff is usually a **zero-copy re-import**.
   - **Separate (discrete dGPU + iGPU, or two discrete GPUs):** distinct memory
     pools across PCIe. Crossing is PCIe P2P (if supported) or a bounce through
     system RAM → usually a **real copy**.

Node split alone forces `DmabufCopy`; the memory axis decides whether that copy can
be elided. This is why a single blit fallback is **mandatory** — separate-memory
topologies cannot always zero-copy no matter how well formats line up.

## Current state (landed: Stages 1–3)

The selection/assembly path is split-aware; the frame path is not yet.

- **Settings** — `render_node` (the "active GPU", pins wgpu) plus the optional
  `scanout_node` override. See `config.base`.
- **Capability probe** — `drm.device/device.capability` runs the *functional* KMS
  probe (`drmModeGetResources` + ≥1 CRTC + ≥1 connector); never trusts sysfs.
- **Resolution** — `native.device/device.select/select.scanout`: render node
  anchored to settings; scanout = render card if itself KMS-capable (desktop,
  `route None`, byte-identical), else the probed KMS card. Explicit `scanout_node`
  = probe-or-verbose-panic, no fallback.
- **Assembly** — `assemble.display` opens the *scanout* card for the `DrmDevice`.

**What does not yet work:** on a split system this brings KMS up but pixels do not
flow — the render frame is never delivered into a scanout-owned buffer. That is
Stage 4.

## Planned refinement: strict node selection + explicit fallback flags

**Status: planned, NOT built.** The landed Stage 3 selection does *silent* discovery —
render falls back to the smithay heuristic/first card when `render_node` doesn't
resolve, and scanout auto-discovers a KMS card whenever the render card isn't
KMS-capable. The intended model is **strict by default, discovery opt-in.**

Principle: if `render_node` (and, when set, `scanout_node`) is configured, *all*
Vulkan/compositor usage must use exactly that — even a render-incapable or non-KMS
choice is honored, and an *unavailable* choice **panics**. Silent substitution of a
different node is never allowed; it must be explicitly enabled.

- **Render node — strict.** Resolve `render_node` → its card. Present on the seat →
  used (KMS-capable or not, render-capable or not; not second-guessed). Unavailable
  (not a present DRM card) → **panic**. Discovery (heuristic card → first card) only
  when `explicit_render_node_fallback = true`.
- **Scanout — strict.** Preferred scanout = `scanout_node` if set, else the render
  node's card. Probe it: KMS-capable → use it; not KMS-capable/unavailable →
  **panic**. Discovery of another KMS card (the "#2" probe-and-rank) only when
  `explicit_scanout_node_fallback = true`.

Consequences:
- **Desktops unaffected** — the render card is KMS-capable, so scanout = it, no flags.
- **Split systems panic by default** with an actionable message ("set `scanout_node`
  or enable `explicit_scanout_node_fallback`"). The nano therefore needs one explicit
  opt-in — matching the "be explicit about non-obvious topology" stance.
- This **supersedes** the earlier "explicit `scanout_node` never falls back" rule:
  fallback is now the flag, defaulting off (same default behavior, now toggleable).

New settings (`config.base` `Environment`), both `#[serde(default)]` bool (omittable,
default `false`, seeded in `default_settings`): `explicit_render_node_fallback`,
`explicit_scanout_node_fallback`. Naming per the user's request; note the mild
inconsistency with `scanout_node` ("scan" vs "scanout") — resolve before building.

Implementation touch points (when built):
1. `config.base` — two bool fields + docs + `default_settings`; `edit.rs` pass-through.
2. `gpu_rank` — `render_fallback()` / `scan_fallback()` accessors.
3. `select.scanout::resolve` — take the two flags; restructure so render resolution
   panics-instead-of-fallback unless `render_fallback`, and scanout treats
   `scanout_node`-or-render-card as the strict preferred, gating the #2 discovery on
   `scan_fallback`. Unifies today's separate explicit/auto scanout branches.
4. `assemble.display` — read the flags from `gpu_rank`, pass to `resolve`.

Optional adjacent hardening (separate decision): also probe the render node for
*render* capability (today only KMS is probed), so a render node pointed at a
display-only device fails here with a clear message instead of later at wgpu pinning.

> **Open question — the two fallback flags (to decide, not settled).** These flags
> add *extra* opt-in functionality; they do not replace the base behavior above.
> Intended semantics (per user):
> - `explicit_scanout_node_fallback` — prefer the explicit `scanout_node`; if it is
>   **not** a compatible (KMS) card, instead of panicking, **continue down the
>   auto-discover path**: first try the selected render node's card, then the #2
>   probe-and-rank fallback. Off (default) = the strict probe-or-panic above.
> - `explicit_render_node_fallback` — analogous for the render node: prefer
>   `render_node`; if unavailable, fall through to the discovery path (heuristic
>   card → first card) instead of panicking. Off (default) = strict panic.
> Decide before building; these supersede the "only gates #2" wording in the bullets
> above (the flag makes a *bad explicit choice* flow into discovery, not a blanket
> gate on discovery).

## Near-term: the nano (single scanout, unified memory) — Stage 4

Scope: **one** scanout device, UMA. Goal: make pixels flow on Orin.

The handoff is per frame: the compositor renders (on `nvgpu`), then the frame must
be a framebuffer the scanout card (`nvidia-drm`) will `AddFB2` + flip on its CRTC.
Two mechanisms, decided at runtime **per (render, scanout) pair**:

1. **Zero-copy import (fast path).** Constrain the scanout framebuffer to a
   format+modifier in the intersection `{render can render into} ∩ {scanout
   advertises as scannable}`; export/import the same dmabuf across the boundary; no
   pixels move. On Orin's UMA this should usually succeed.
2. **De-modifier blit (floor).** When the intersection is empty (display wants
   linear / block-linear, render is tiled/DCC), or import is rejected: allocate a
   scanout-compatible buffer on the scanout card's GBM and GPU-blit into it.

Implementation touch points:
- **Render-side allocation** — today the GBM allocator (`drm.gbm/gbm.alloc`) and
  the renderer are paired to `primary_gpu`; the scanout GBM now lives on a
  different card. Either allocate the scanout FB on the scanout card and render
  into an imported view (preferred, reuses the existing GBM→wgpu import), or
  allocate on render and import into scanout.
- **Executor** — `native.render/render.execute` gains a `CopyRoute::DmabufCopy`
  arm: attempt import, fall back to blit, then commit on the scanout CRTC.
- **Modifier negotiation** is the same capability the internal bridge already needs
  (see [[dmabuf-bridge-modifier-divergence]] in memory / `BRIDGE_PARALLAX.md`):
  if wgpu will not let us pin the intersection modifier, we are structurally on the
  blit floor. **So blit lands first; import is the optimization.**

Gating facts from the board (before/while building): `card* → driver` map,
`nvidia-drm.modeset=1` present, and the scanout connector's advertised
format/modifier list (`drm_info`). If no KMS card exists, Stage 3 already
verbose-panics telling the user to set `modeset=1`.

## Deferred: the hardware zoo

A taxonomy, roughly increasing in difficulty:

- **(A) Unified SoC, split drivers — Orin.** One GPU, one memory, render-only node
  + display-only KMS card. *Handled by Stage 4 above.*
- **(B) Discrete PRIME laptop.** iGPU owns the panel, dGPU renders; separate
  memory. Single scanout device but the copy is often *real*. Stage 4's blit floor
  covers it; the import fast path frequently won't apply.
- **(C) Multi-GPU, monitor per GPU.** Two KMS cards, each owning different
  connectors. **This is the real generalization** — see CRTC management below.
- **(D) Reverse-PRIME / muxless outputs.** A connector physically wired to the
  dGPU while compositing on the iGPU (or vice-versa) — same as (C) but the "second
  scanout device" is a dGPU. Same machinery as (C).
- **(E) Software/CPU render clients.** Orthogonal: SHM (CPU-rendered) client
  surfaces bypass the dmabuf bridge entirely (they are *uploaded*, always linear,
  modifier-agnostic). Only *dmabuf* clients hit modifier negotiation — and a dmabuf
  client may have rendered on yet another GPU, so there are two crossings:
  `client-GPU → render node` (bridge #1, exists today) and `render → scanout`
  (bridge #2, Stage 4). llvmpipe/lavapipe appearing in the adapter list is a
  *fallback ICD*, not the compositor rendering on CPU.

### How CRTC management links the process (the (C) case)

This is the crux of monitor-on-GPU-A + monitor-on-GPU-B, and why it is a structural
change rather than a bigger copy.

In DRM the display pipeline is **per card**: each card enumerates its own CRTCs,
encoders, connectors, and planes. A connector belongs to exactly one card; a CRTC
can only drive connectors on **its own** card. You cannot borrow card-A's CRTC to
light card-B's port. Therefore:

- **The scanout device for an output is not a choice — it is dictated by which card
  enumerates that output's connector.** "Monitor on GPU-B" literally means the
  connector, its CRTC, and its planes all live on card-B's `DrmDevice`.
- Driving output-B is a **fully independent pipeline on card-B**: claim a CRTC on
  card-B → attach card-B's connector → allocate the scanout FB via card-B's GBM →
  `atomic_commit` on card-B → receive card-B's own vblank/page-flip events.
- So "resolve *the* scanout card" (Stage 3, single) generalizes to **per-connector
  scanout binding**: `scanout_device(output) = the card that enumerates output's
  connector`, and `route(output) = topology.route(render_node, that card)`. Some
  outputs come out `None` (the render card's own monitors), some `DmabufCopy` (the
  other card's monitors) — a *mix within one session*.

Concrete restructuring this implies:
1. **Enumeration** moves from "the DRM device" to "a set of KMS cards"; connectors
   are grouped by owning card.
2. **`assemble.display`** produces **one output pipeline per (card, connector)**,
   each owning its own CRTC/plane/GBM — not a single `DrmDevice` + connector.
3. **The frame loop** composites once (single wgpu context on the render node),
   then, per output: extract that output's region, ensure it is in a buffer the
   output's card can scan out (route `None` → direct; `DmabufCopy` → import/blit
   onto that card), and commit on **that card's** CRTC.
4. **Presentation/pacing** gets one vblank clock **per card**. Outputs on different
   GPUs flip on independent timelines; frame pacing needs a per-device flip handler
   and an explicit policy for whether/how to align them (generally: pace per
   device, don't force cross-GPU vsync lock).
5. **Session/seat** must open and `become-master` on **every** KMS card, not one —
   libseat device management, DRM master, and VT-switch pause/resume all become
   per-card.

y5 already drives a **single** output today, so (C) stacks two generalizations:
first single-GPU **multi-output** (multiple CRTCs on one card), then
multi-*device* multi-output (CRTCs across cards). The topology crates
(`role`/`route`) are already per-node, which is the seam this hangs off.

## The negotiate-vs-blit decision (applies to all cases)

Not global, not compile-time: decided **per (render, scanout) pair, per output, per
buffer** at runtime.

- **Negotiate first:** intersect render-capable and scanout-scannable modifiers;
  pin the render/scanout FB to a common one; import. Zero-copy when it works
  (UMA/Orin, some PRIME).
- **Blit floor:** always-correct fallback; mandatory because separate-memory
  topologies (B/C/D) frequently cannot zero-copy.

The negotiation capability is shared with the existing internal bridge's
modifier gap — landing it once benefits both crossings. If wgpu cannot pin a chosen
DRM modifier, everything falls back to blit regardless, which is the second reason
blit is built first.

## Open decisions (to resolve before the deferred (C) work)

- Single shared wgpu context + cross-device import for foreign outputs (y5's
  current model) **vs** one renderer per GPU (render each output locally, no
  bridge #2 for local outputs, but multiple GPU contexts to manage).
- Per-card flip pacing policy (independent vs best-effort alignment).
- Whether to expose per-output scanout binding in settings or fully auto-derive it
  from connector ownership (leaning auto-derive; connector ownership is not a
  user choice).

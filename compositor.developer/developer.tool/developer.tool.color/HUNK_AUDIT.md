# Hunk audit — `7aff76a6` (A), `3a6eb2d4` (B), `0e933117` (C)

Per-hunk justification for the three commits on `upstream-integration`. **Evidence** is what
was observed, not what a change was meant to do.

**Status:** the removals are DONE, in `0e933117`. What follows is the current state, not a
proposal. Several items the first revision of this audit filed as speculative have since been
proven or disproven by measurement — where a verdict changed, the change is stated.

---

## A — `7aff76a6` "unsupported color modifiers" — keep entirely

One statement: **advertise exactly what the compositing renderer can import.** Confirmed on
two machines — the fp16 blank-window case, and vkcube on the laptop, which could not import
its own texture into our Vulkan until this landed.

| Hunk | Justification |
|---|---|
| `query.rs` `sampleable()` vs `renderable()` | **The vkcube fix.** The old gate demanded COLOR_ATTACHMENT, which drops formats that import and sample fine. A client buffer is only ever sampled. |
| `modifier.rs` `modifiers_with()` + `import_formats()` | A device can list a modifier and still not sample through it; `VkFormatProperties` cannot say which. The per-modifier answer is the only binding one — and C extends the same rule to the render side. |
| `table.rs` (new, 234 ln) | Exhaustive fourcc table, **no wildcard arm**. The hand-kept list went stale twice (10-bit, then fp16), each time surfacing as a blank window. A new `Fourcc` is now a compile error until someone decides. |
| `renderer/import.rs` `dmabuf_formats()` | Hand-written list → `import_formats()`. Advertised set and import decision derive from one table and cannot drift. |
| `display.output` `advertise()` | Publishing more than the renderer takes = accepted at validation (GLES), failed at draw, every frame, blank window. |
| `memory.import` opaque via table | The old 4-format `matches!` silently missed every X-format added after it (`Xbgr16161616f`, `Xrgb1555`, …) → windows blending out transparent. |
| `memory.import` `dmabuf_bytes()` + size warn | Log-only. Predicts a black band when a client buffer is shorter than `requirements.size`. Still never observed firing. |
| `wire.rs` refusal | A client that gets `failed()` can fall back; one with a blank window cannot. |
| `entry.rs` init reorder (~35 ln) | **Load-bearing.** `register_dmabuf` builds the feedback; if nothing has published, `advertise()` degrades to the raw EGL list — the exact bug. Riskiest hunk in A, and required. |

**Separable:** the fp16 colour policy (`set_color_managed`, `expressible()`, the
`worker_modifiers` gate, `may_import()`, the `hdr_active` hoist). Defensible on its own terms
but not required by the vkcube evidence. Drop only as a unit.

---

## B — `3a6eb2d4` "experimental" — what survived

### Proven — kept

| Hunk | Evidence |
|---|---|
| `submit.rs` `in_flight_textures.extend(acquires)` | **Reproduced.** `pending_acquires` is not a pin; a barrier is recorded whether or not damage caused a draw. An undamaged surface whose buffer died in the same frame was freed while the submitted command buffer still referenced it → Xid 31 `FAULT_PTE`. Repro: cycle one dmabuf window per format, ~1 min. |
| `blur.rs` negotiated modifiers | **Diagnosed.** Allocated with no modifier list; driver chose `0x0300000000E08014`, outside the importable six. GLES imported it via EGLImage so blits succeeded, but `vkCreateImage` returned `FORMAT_NOT_SUPPORTED`, the view had no format features, and the draw proceeded — a garbage descriptor sampled alongside **unrelated windows**. |
| allocator: implicit entry points private, `NoNegotiatedModifier` | Structural enforcement of the above. The old code fell through to implicit on an empty list *or any failure*. |
| `surface_modifiers()` + 5 call sites | Adds the compositor's own importable set to `gles ∩ wgpu`. Omitting that term **is** the blur bug, at five more sites. |
| `color_formats`: B-first at 10 bits | **Measured** (NVIDIA 595.80): the plane offers `AR15 AR24 XR15 XR24` but at 10 bits only `AB30 XB30`. An R-first-only ladder misses every rung and drops silently to 8-bit. |
| `worker.format` follows `scanout_fourcc()` | Same measurement. |
| ~~vendored smithay cursor patch~~ | **REMOVED IN FULL** — see below. |
| `bind.rs` `TRANSFER_SRC` | `capture.blit`/`mipgen` use a bound target as `srcImage` (VUID-vkCmdBlitImage-srcImage-00219). NVIDIA executed it anyway, so it only showed under validation. `vkfmt` measured that every colour-attachable modifier here also carries TRANSFER_SRC. |
| `narrow_to_composite` + renderable publish | **Verdict reversed — this was filed as the riskiest item to drop.** It is what makes a cross-vendor split run at all: it forces a modifier both vendors understand, where before `vkCreateImage` failed every frame with `INVALID_DRM_FORMAT_MODIFIER_PLANE_LAYOUT_EXT`. C then made it strict. |

### Removed in `0e933117`

| Hunk | Why |
|---|---|
| `wait_implicit_fence()` (CPU poll) | "Measured to cost nothing" is not "proven to fix something". Nothing was ever observed that it corrected, and the semaphore-based version it stood in for is unavailable here anyway (NVIDIA rejects `SYNC_FD` semaphore import). Removed with no residue — `import.rs` is byte-identical to commit A. **Given up knowingly: implicit-sync clients (most of them) are again sampled with nothing ordering our read against the producer's write.** |
| `frame.rs` `clear()` accumulating | Reverted to the assignment. It was defended as "one line, strictly safer" — but the multi-`clear()`-per-frame case it guarded was never observed, so it failed the same test as everything else here. Being small is not evidence. |
| vendored smithay cursor patch (both halves) | The `fill(0)` targeted a stale-margin artifact never reproduced on this hardware. The row/byte clamp guarded a panic from a client-controlled shm stride that was also never observed. Neither met the bar, and a vendored delta is the most expensive kind to carry — it conflicts on every smithay update. `vendor/.../compositor/mod.rs` is now byte-identical to the vendored upstream. **Given up knowingly: upstream slices by `src_stride`/`src_height` unchecked, so an over-tall or over-padded client cursor can still index past the mapping.** |
| queue-family FOREIGN acquire/release pairing — `record_composition`'s `post` param, `record_pending_releases`, the `GENERAL` resting layout, the fresh-import push into `pending_acquires`, both submit call sites, both self-test closures | Written against the `UNSUPPORTED_KIND` theory; never fixed the symptom; changed the resting layout of **every** imported texture on **every** frame. Spec-correct in principle — a queue-family transfer is defined only as a pair — but unproven work with that blast radius does not stay. It also shipped a defect: four closures passed to a three-closure function, invisible because `vulkan_self_test` is `cfg`-gated off. |

---

## C — `0e933117` — the modifier-legality fix

| Hunk | Justification |
|---|---|
| `render_formats` filters per modifier | **Measured on two NVIDIA GPUs:** LINEAR carries `SAMPLED TRANSFER_SRC TRANSFER_DST BLIT_SRC BLIT_DST` and **no COLOR_ATTACHMENT**. The unfiltered version published it as a render target anyway, and on a cross-vendor split the intersection landed exactly there — so we rendered into an unsanctioned modifier every frame. Permitted by the driver, undefined by the spec, concurrent with the Xid faults. |
| `narrow_to_composite` aborts on empty intersection | The old widening fallback **was** the corrupted path: returning the EGL set with INVALID is what let the swapchain take a modifier the composite cannot colour-attach. Failing at startup with both set sizes named beats rendering undefined pixels. |
| INVALID dropped explicitly | It could not survive a Vulkan-sourced intersection anyway, but that guarantee lived in another crate. Now local. The implicit path remains on the GLES/unpublished branch, where the composite **is** the allocating device and re-derives the layout itself. |
| both halves logged per fourcc | `SURVIVES` / `dropped`, with a marker on any non-LINEAR survivor. This is what produced the finding that **zero** tiled modifiers survive on the laptop. |

**Known consequence:** cross-vendor `renderer = vulkan` now refuses to start. Deliberate.
`renderer = "gles"` composites on the scanout device, renders into Intel-tiled buffers
directly, and X_TILED is async-flip capable — likely the better laptop experience today.

---

## Verdicts that changed, and why

Worth keeping visible, because two of them were confidently wrong.

1. **`narrow_to_composite`: "known risk, drop" → keep, essential.** It was filed as the only
   item with a hardware failure. It is the thing that makes cross-vendor work; the failure was
   a *different* bug (sourcing) that C fixed.
2. **"NVIDIA reports LINEAR as SAMPLED-only" → retracted → reinstated.** The retraction argued
   "the laptop renders into LINEAR, therefore it is supported". Permitted is not supported.
   `vkfmt` on the laptop settled it: no COLOR_ATTACHMENT there either.
3. **`wait_implicit_fence`: "prime FPS suspect, drop" → keep.** Removing it changed nothing
   measurable. The arithmetic said so before the measurement did — one blown 8 ms budget gives
   ≤125fps, not a hard 60 — and that should have outranked the hypothesis.

---

## Defects this audit found (all fixed)

1. **Four closures passed to a three-closure `record_composition`** in `vulkan_self_test`.
   Compiled only because the function is behind `#[cfg(feature = "renderer-vulkan")]`, which
   is off — dead code that broke the moment the feature was enabled. Now moot: the `post`
   parameter is gone, and the feature-gated path is checked in CI-equivalent by building with
   `--features renderer-vulkan`.
2. `record_pending_releases` doc described a superseded design and a first-frame gap that no
   longer existed. Removed with the function.
3. `split_device` doc described a topology gate the code no longer used.
4. `import_sync_file` leaked an fd on its failure path (`into_raw_fd` before a call that only
   takes ownership on success). Pre-existing, on the explicit-sync path. **Fixed and kept.**

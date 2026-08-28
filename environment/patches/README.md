# Patches carried out of tree

Changes to `vendor/*` that we have decided **not** to apply, kept here so the work and
the reasoning survive. Everything in `vendor/` that is not listed here is applied and is
part of the build (see the top-level `CLAUDE.md`: vendored forks are patched in-tree and
are part of the codebase).

A patch lands here when the fix is real but the cost of carrying it outweighs what it
buys. Deleting it would throw away the analysis; leaving it applied would mean
maintaining code we cannot justify.

Apply one with:

```
git apply environment/patches/<name>.patch
```

---

## `naga-spv-layout-strip.patch`

**Fixes:** `VUID-StandaloneSpirv-None-10684` — explicit layout decorations
(`Offset`, `ArrayStride`, `MatrixStride`) on types that a non-laid-out storage class
also uses.

**Includes naga's snapshot tests.** The pass runs on every module naga emits, so its
62 `tests/out/spv/*.spvasm` snapshots record the stripped output. They are in the patch
rather than left in `vendor/`: keeping them behind would leave stock naga failing its
own tests, and splitting them out would mean the patch no longer applies cleanly. The
rule for anything in this directory is that `vendor/` matches upstream when the patch is
off, and the patch alone is what makes it not.

**Why naga produces it.** Types are interned by handle alone
(`LookupType::Handle(Handle<crate::Type>)`, `spv/mod.rs`), so one `OpTypeStruct` is
shared across every storage class, and `decorate_struct_member` (`spv/writer.rs`)
decorates unconditionally with no storage-class check. A struct used in a uniform or
push-constant block and *also* as a function-local therefore carries layout decorations
where SPIR-V permits none. Explicit layout is legal only in `Uniform`, `StorageBuffer`,
`PushConstant` and `PhysicalStorageBuffer`.

**Why we do not carry the fix.**

- The decoration is **inert where it is illegal**. `Function`/`Private`/`Workgroup` have
  no defined memory layout — the driver uses registers or scratch and never consults it.
  The laid-out uses of the same type keep their correct decorations, so buffer addressing
  is unaffected. Invalid, not wrong.
- **The SPIR-V is not new; the rule is.** naga has emitted this shape for years and only
  the validation layer's diagnosis is recent.
- **Upstream ships it knowingly.** `wgpu-hal/src/vulkan/instance.rs` suppresses this exact
  VUID in its debug callback — *"This is a bug. To prevent massive noise in the tests,
  lets suppress it for now"* — pointing at <https://github.com/gfx-rs/wgpu/issues/7696>.
- **The cost is 667 lines of untested SPIR-V word-stream surgery** running unconditionally
  on every module naga emits, including a split path that clones types, retypes access
  chains to a fixpoint and inserts `OpCopyLogical`. That path is active at the SPIR-V 1.6
  wgpu-hal requests on a Vulkan 1.3 device.
- **It is incomplete anyway.** See the header inside the patch.

**What we did instead.** The only real cost was log noise drowning sync validation, so the
VUID is filtered in `environment/vk_layer_settings.txt`. That is a line in a file we own
rather than a rewrite of every shader.

**When to revisit.** If a driver ever rejects a module, or if SPIR-V tooling that actually
validates (`spirv-val`, `spirv-opt`, GPU-AV instrumentation) refuses to process our
shaders. At that point the right move is finishing the pass — including `ColMajor` and
`Block` — and upstreaming it to wgpu#7696, not carrying it here.

**Risk notes if it is ever reapplied.** The strip half is structurally safe: `layout` and
`plain` come from the same reachability walk over the same edges, so a missing edge drops
a type from *both* sets and a type absent from `plain` is never stripped. Two paths could
still over-strip, neither reachable from WGSL: a laid-out variable whose `OpTypePointer`
does not resolve is silently skipped, and the walk follows struct/array members but not
pointers nested inside a struct (`buffer_reference`/`PhysicalStorageBuffer`).

---
name: logging
description: How to log in the y5 compositor. Use whenever you add, change, or migrate logging — any time you'd reach for a log/print statement, "add a log", debug output, `tracing::*!`, `println!`, or a fatal `panic!` with a message. y5 has its own tracing-free structured logging; this skill is the source of truth for using it.
---

# y5 logging

y5 has its **own** structured logging system. **Never use `tracing`, `log`, or `println!`/
`eprintln!` for diagnostics in compositor code.** Use the macros from
`compositor_model_debug_instance_record`. Full reference: `document/LOGGING.md`.

## The rule

- Levels: `error!`, `warn!`, `info!`, `trace!` — same `format!` args as `tracing`.
- Fatal: `abort!("msg {x}")` — logs at Error **and** panics (synchronously flushed first).
- No `tracing` / `log` / `tracing-subscriber` deps in new/changed crates. If a crate you touch
  still imports them only for logging, migrate the call sites and drop the deps.

## Wiring a crate to log (one-time per crate)

The `add-crate` template already inserts this. If a crate lacks it, add to its **`lib.rs`**
(after any leading `#![...]` inner attributes):

```rust
#[macro_use]
extern crate compositor_model_debug_instance_record;
```

and ensure its `Cargo.toml` has (it resolves via the workspace links — run `./link.all.sh` if
you just added the dep to a workspace that didn't have it):

```toml
compositor_model_debug_instance_record = { workspace = true }
```

Then call the macros from any module in the crate — no per-module `use` needed. The crate name
and function path are attached automatically as filter tags.

## Migrating `tracing`

`tracing::info!(...)` → `info!(...)` (likewise `error!`/`warn!`/`trace!`). Confirm the two
`lib.rs` lines above exist, then remove `tracing` + `tracing-subscriber` from the crate's
`Cargo.toml`. Grep the crate for `tracing::` to be sure none remain.

## Levels: two independent controls

- **Compile-time**: cargo features `error/warn/info/trace` on the record crate (set by the
  top-level `execute` dep). A disabled level compiles to nothing. `abort!` is never stripped.
- **Runtime**: `COMPOSITOR_LOG_LEVEL` (e.g. `info,warn,error,trace`) selects what is emitted
  among compiled-in levels.

## Viewing logs

Records stream over gRPC (`/tmp/y5-compositor-logs.sock`) to the **`compositor.developer/developer.tool`**
Tauri viewer (filter by crate/function/level, timeline, presets, dumps). Run the compositor with
`COMPOSITOR_LOG_LEVEL` set, then `cd compositor.developer/developer.tool/developer.tool.window/logs && npm run tauri dev`. They also
print dmesg-style to the compositor's stderr.

## Do NOT

- Do not add `tracing`/`log` to `Cargo.toml`.
- Do not `println!`/`eprintln!` for diagnostics (use `info!`/`trace!`).
- Do not `panic!("msg")` for a logged fatal — use `abort!("msg")`.
- Do not hand-write a per-crate logger — `instance!()` is the only mechanism.

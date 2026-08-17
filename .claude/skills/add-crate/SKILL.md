---
name: add-crate
description: Add a new member crate to a Cargo workspace in this repo using the `y5-template` binary. Use whenever the user asks to add/create/scaffold a new crate, module, or workspace member (e.g. "add a crate under compositor.window", "scaffold action.foo"). Handles the chain-prefix naming convention and non-interactive invocation.
---

# add-crate

Scaffold a new member crate into a Cargo workspace using `y5-template` (on PATH at
`~/.cargo/bin/y5-template`; source in `environment/toolkit/y5-template/`). Do NOT hand-create
`Cargo.toml`/`lib.rs` by hand — use the tool so naming and the template stay correct.

**ALWAYS run `compositor.workspace/link.all.sh` from the repo root after creating a crate** (step 5) —
the crate is not wired into the workspaces until you do.

## The naming convention (chain-prefix)

A crate is created **exactly two levels** below a workspace root, and each level's
directory name chains off its parent: `{parent_tail}.{own_segment}`.

```
compositor/                  workspace root  (Cargo.toml [workspace] members glob has ≥2 `*`)
  compositor.action/         L0   (root_tail "compositor" + "action")
    action.window/           L1 = the target directory
      window.{Name}/         L2 = what the tool CREATES
```

- **root_tail** = last dot-segment of the workspace dir name (`compositor` → `compositor`).
- A valid **L0** is `{root_tail}.{seg}`, a valid **L1** is `{seg}.{sub}`.
- The created dir is `{L1_own_segment}.{Name}` and must not already exist.

## Steps

1. **Find valid targets** (the L1 dirs you can add into):
   ```
   y5-template --scan /workspace --list
   ```
   This prints every valid `<workspace> › <L0> › <L1>` with its absolute path.

2. **Create the crate non-interactively.** The picker needs a TTY, so use `--dir`
   (which skips the picker when the dir validates) and pipe the Name to stdin:
   ```
   printf 'NAME\n' | y5-template --dir /workspace/compositor/compositor.action/action.window
   ```
   - `NAME` becomes the crate suffix → creates `window.NAME/`.
   - **Batch:** `printf 'a, b, c\n' | y5-template --dir <L1_DIR>` creates `window.a`,
     `window.b`, `window.c` atomically (aborts if any already exists).
   - **Extra template vars:** if the chosen template declares vars beyond `Name`,
     they are prompted after Name — feed them as additional lines:
     `printf 'NAME\nval1\nval2\n' | y5-template --dir <L1_DIR>`.
     Check first with: `y5-template --template <NAME> --help` is not enough — inspect
     the template under `<workspace>/y5.template/<tpl>/` for `$${var}$$` placeholders.
   - `--template <NAME>` selects a non-default template (default is `default`).

3. **Need a new L0 or L1 first?** The `+ L0/L1` bootstrap only works through the
   interactive picker, so non-interactively just create the chain-prefixed dirs
   yourself, then run step 2:
   ```
   mkdir -p /workspace/compositor/compositor.<L0>/<L0>.<L1>
   printf 'NAME\n' | y5-template --dir /workspace/compositor/compositor.<L0>/<L0>.<L1>
   ```

4. **Verify**: confirm `{L1}.{Name}/` exists with the template files, and that the
   workspace `Cargo.toml` member glob already covers it (it does if the glob is
   `compositor.<L0>/*/*`-style — no manual `members` edit needed).

   The template's `lib.rs` is already SHELL-conforming (`extern crate` + `pub mod`) —
   **keep it that way.** Code goes in the generated `<module>.rs` beside it; a `fn`,
   `struct`, `impl` or `const` written into `lib.rs` fails the lint, and that rule has
   no allowlist. Re-export from `lib.rs` with `pub use <module>::…` if callers should
   see an item at the crate root.

5. **Write its `crate.json`.** `Cargo.toml` is a GENERATED artifact — gitignored, and
   overwritten on every build — so a crate declares its dependencies in `crate.json`
   beside `lib.rs`. It is JSONC, so a `//` comment above a dependency is kept and
   re-emitted above that line in the generated manifest:
   ```jsonc
   {
     "deps": [
       // Developer logging — provides error!/warn!/info!/trace!/abort!.
       "compositor_model_debug_instance_record",
       "smithay"
     ]
   }
   ```
   Names only. A crate never states a version, a feature or a manifest shape — those
   live in `compositor.workspace/vendor.catalog.json` (external versions/features) and
   `compositor.workspace/workspace.catalog.json` (per-crate features, `[[bin]]`, build
   scripts, …).

   Only add what the crate actually uses: an unused dependency FAILS the lint
   (`deps-unused`), and a dependency that is genuinely needed but never named in the
   source — e.g. spliced in by `tonic::include_proto!` — goes in `keep` with a reason.

6. **Regenerate.** `environment/build.sh` and `environment/check.sh` do this for you
   before invoking cargo, so usually there is nothing to run. To refresh the tree for
   an editor without building: `compositor.workspace/link.all.sh` (which now just generates + lints).

## Template variables (for reference)

For `compositor/compositor.action/action.window` + `Name = handle`:

| variable                      | value                             |
|-------------------------------|-----------------------------------|
| `workspace_name`              | `compositor`                      |
| `L0` / `L1`                   | `action` / `window`               |
| `Name`                        | `handle`                          |
| `fully_qualified_crate_name`  | `compositor_action_window_handle` |
| `fully_qualified_module_name` | `handle`                          |

## Notes

- `--scan` defaults to `$ZED_WORKTREE_ROOT` then cwd; pass `--scan /workspace`
  explicitly when running from elsewhere.
- In Zed, humans use the picker via `alt-n` (default template) / `alt-shift-n`
  (named); that path also pins the L1 of the currently-open file. The CLI `--dir`
  flow above is the equivalent for non-interactive/agent use.
- Full reference: `environment/toolkit/y5-template/`.

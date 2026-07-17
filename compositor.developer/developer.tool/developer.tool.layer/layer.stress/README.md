# layer.stress — Wayland layer-shell + foreign-toplevel stress harness

A two-process developer tool for exercising the y5 compositor's **layer-shell**
(`zwlr_layer_shell_v1`) support and its **foreign-toplevel** protocols
(`zwlr_foreign_toplevel_manager_v1` + `ext_foreign_toplevel_list_v1`) — one tool in place of the
zoo of daemons (waybar, eww, swaybg, wlogout, …) that each cover only a slice.

Sibling of `../../developer.tool.window/window.stress` (which does the same for xdg toplevels).
Standalone crate: its own `Cargo.lock`, crates.io deps only, **not** in `link.all.sh`.

```
layer-stress-controller   GUI window with clickable buttons; spawns + drives the subject
layer-stress-subject      the layer surface under test; reconfigures itself on command
```

The **controller** opens a button panel (grouped by dimension) and spawns the **subject** as a
child process, forwarding one [`Command`] line per click to the subject's stdin. The **subject**
owns a single `zwlr_layer_surface_v1` (via sctk's `wlr_layer` helper) and drives every
layer-shell dimension live; it also binds **both** foreign-toplevel managers, shows what each
advertises, and can issue the wlr control requests. It renders a live state overlay, a pointer
crosshair, a click marker (to see how input regions route clicks) and keyboard focus / last key.

## Build & run

```bash
cargo build --release
# Run against any wlr-layer-shell compositor (point WAYLAND_DISPLAY at a nested y5):
./target/release/layer-stress-controller
```

The controller finds `layer-stress-subject` next to its own executable. Diagnostics from both
processes print to stderr, tagged `[controller ...]` / `[subject ...]`.

Drive the subject directly (scripting / CI repros) by piping commands:

```bash
( printf 'layer overlay\nanchor-top\nanchor-left\nexcl 40\nmargin 8 8 8 8\n';
  printf 'kbd exclusive\npopup-add\ninput holes\nanimate on\n'; sleep 1;
  printf 'quit\n' ) | ./target/release/layer-stress-subject
```

Headless check of the controller's command surface (no Wayland, no child):

```bash
./target/release/layer-stress-controller --selftest
```

## Protocol selection

The subject only binds the optional globals you allow. The controller's `PROTOCOLS` row toggles
these; press `RESPAWN` to relaunch the subject with the new set. Equivalent flags:

```
--no-viewporter  --no-fractional-scale  --no-foreign-wlr  --no-foreign-ext
```

`zwlr_layer_shell_v1` itself is required (the subject exits if it is absent).

## Command vocabulary (controller buttons → subject)

| Group | Commands |
| ----- | -------- |
| Layer | `layer background\|bottom\|top\|overlay` (live, via `set_layer`) |
| Anchor | `anchor-top`, `anchor-bottom`, `anchor-left`, `anchor-right` (toggles), `anchor-all`, `anchor-none` |
| Exclusive zone | `excl N` (buttons: `-1`, `0`, ±8 stepper) |
| Margins | `margin T R B L` (per-edge ± steppers) |
| Keyboard | `kbd none\|exclusive\|ondemand` |
| Size | `size W H` (`0` in a dimension = stretch along the anchored edge) |
| Popup | `popup-add`, `popup-close`, `popup-anchor A`, `popup-gravity A`, `popup-off X Y`, `popup-size W H`, `popup-move DX DY` |
| Input region | `input full\|none\|circle\|holes` |
| Opaque region | `opaque full\|none` |
| Output | `output-cycle` (recreate on the next `wl_output`) |
| Buffer scale | `scale-normal`, `fs-honor`, `fs-ignore`, `dpi-honor`, `dpi-ignore`, `dpi-scale N` |
| Appearance | `transparent on\|off`, `animate on\|off` |
| Foreign (wlr) | `foreign-sel ±1`, `foreign-activate`, `foreign-close`, `foreign-max on\|off`, `foreign-min on\|off`, `foreign-full on\|off` |
| Lifecycle | `recreate`, `quit` |

The subject overlay shows, top to bottom: layer / anchor mask / exclusive zone / margins;
requested vs **configured** size, keyboard mode + focus + last key; buffer px + scale + output /
fractional scale; input & opaque region modes, output index, popup count, transparency &
animation; then the **FOREIGN** panel — `wlr` and `ext` advertisement (version or `absent`), the
selected index, and the two advertised toplevel lists side by side (title, app_id, and — wlr
only — decoded states `M`/`m`/`A`/`F`).

## What to look for

- **Anchors / exclusive zone / margins** — the surface snaps to the chosen edges; a positive
  `excl` reserves space so other surfaces reflow around it; `-1` lets it ignore other exclusive
  zones; margins offset it from its anchored edges.
- **Layer level** — `overlay` stacks above everything (incl. fullscreen), `background` below
  windows. Toggling live via `set_layer` exercises dynamic reconfiguration.
- **Keyboard interactivity** — with `exclusive` the subject receives keys (watch `FOCUS YES` +
  `KEY …`); with `none` it never gets focus; `ondemand` grants focus on click.
- **Input regions** — `none` makes the whole surface click-through (no click marker appears,
  cursor reaches what's behind); `holes` passes clicks in the centre through but not the border;
  `circle` accepts only the disc. The click marker confirms exactly which pixels were delivered.
- **Popups** — `popup-add` opens an xdg_popup parented to the layer surface; `popup-move` /
  anchor / gravity push it around and (near screen edges) test constraint handling.
- **Multiple outputs** — `output-cycle` moves the surface between monitors; pan/zoom the y5
  world and confirm the layer surface stays pinned to its output.
- **Fractional / HiDPI** — `fs-*` / `dpi-*`: confirm layout follows the logical (configured)
  size, not raw buffer pixels.
- **Foreign toplevel** — open a couple of ordinary windows; both the `wlr` and `ext` lists
  should populate identically (advertisement). `foreign-sel` + `foreign-activate` / `-close` /
  `-max` / `-min` / `-full` drive the wlr control requests and should move focus / state on the
  targeted window. With the compositor's `protocol_foreign` preference **disabled**, both
  managers still bind (non-`absent`) but the lists stay empty — that is the opt-in gate working.

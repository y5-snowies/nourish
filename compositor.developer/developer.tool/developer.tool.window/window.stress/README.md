# window.stress — Wayland window-layout stress harness

A two-process developer tool for reproducing/​isolating window-layout bugs in the y5
compositor (decorations out of sync, buffers committed at the wrong size, popups/subsurfaces
shifting windows out of bounds, scale handling, single-pixel rendering).

Standalone crate: its own `Cargo.lock`, crates.io deps only, **not** in `link.all.sh`.

```
window-stress-controller   GUI window with clickable buttons; spawns + drives the subject
window-stress-subject      the experimental window under test; misbehaves on command
```

The **controller** opens a button panel (grouped by scenario), spawns the **subject** as a
child process, and forwards one [`Command`] line per click to the subject's stdin. The
**subject** drives **raw** xdg-shell / xdg-decoration / viewporter / fractional-scale /
single-pixel objects (sctk's high-level `Window` auto-acks configures, which would forbid the
ack-abuse cases) and renders a live state overlay plus a **crosshair at the pointer location**
on whichever of its surfaces the pointer is over.

## Build & run

```bash
cargo build --release
# Run against any Wayland compositor (point WAYLAND_DISPLAY at the target, e.g. a nested y5):
"$(cargo metadata --no-deps --format-version 1 | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')"/release/window-stress-controller
```

> The repo's `.cargo/config.toml` pins ONE `target-dir` for the whole checkout, and it applies
> here too — the binaries land in `compositor.kernel/kernel.loader/target/release/`, not in a
> `target/` beside this crate. The `cargo metadata` line above resolves it wherever it is.

The controller finds `window-stress-subject` next to its own executable. Diagnostics from
both processes print to stderr, tagged `[controller ...]` / `[subject ...]`.

Drive the subject directly (scripting / CI repros) by piping commands:

```bash
( printf 'deco-mode client\n'; sleep .5; printf 'popup-add\n'; sleep .5;
  printf 'popup-move 400 0\n'; sleep .5; printf 'quit\n' ) | ./target/release/window-stress-subject
```

Headless check of the controller's command surface (no Wayland, no child):

```bash
./target/release/window-stress-controller --selftest
```

## Protocol selection

The subject only binds the optional globals you allow. The controller's `PROTOCOLS` row
toggles these; press `RESPAWN` to relaunch the subject with the new set. Equivalent flags:

```
--no-decoration  --no-viewporter  --no-fractional-scale  --no-single-pixel
```

## Command vocabulary (controller buttons → subject)

| Group | Commands |
| ----- | -------- |
| Decoration | `deco-mode server|client|none`, `deco-ignore`, `deco-badsize` |
| Buffer size | `buf-agreed`, `buf-delta N`, `buf-zero`, `buf-noack`, `buf-preack`, `geo-mismatch` |
| Popup | `popup-add/nest/close`, `popup-anchor`, `popup-gravity`, `popup-off X Y`, `popup-size W H`, `popup-move DX DY` |
| Subsurface | `sub-add/nest/remove`, `sub-move DX DY`, `sub-sync`, `sub-desync` |
| Viewporter | `vp-dest W H`, `vp-dest-delta N`, `vp-src X Y W H`, `vp-animate on|off`, `vp-unset`, `vp-bad` |
| Fractional scale | `fs-honor`, `fs-ignore`, `fs-scale N`, `fs-noviewport`, `fs-mismatch` |
| DPI / integer scale | `dpi-honor`, `dpi-ignore`, `dpi-scale N`, `dpi-nondiv`, `dpi-mismatch`, `dpi-zero` |
| Single-pixel buffer | `sp-fill R G B A`, `sp-sub R G B A`, `sp-noviewport` |
| Toplevel icon | `icon-name NAME`, `icon-buffer EDGE...`, `icon-both NAME`, `icon-clear` |
| Lifecycle | `map`, `unmap`, `mapcycle on|off`, `size W H`, `quit` |

### Toplevel icon (`xdg_toplevel_icon_v1`)

The protocol has two independent halves, and a compositor has to handle both plus their
absence — three variants that look identical from the outside:

| Variant | Command | What the compositor must do |
| ------- | ------- | --------------------------- |
| Stock name | `icon-name firefox` | resolve the string through the icon theme |
| Pixel buffers | `icon-buffer 64` | read pixels out of the client's `wl_shm` pool |
| Neither | `icon-clear` | fall back to whatever it infers about the app itself |

Two more that catch the interesting mistakes:

- `icon-both firefox` sets a name AND a buffer on one icon object — whichever a consumer
  prefers, it has to pick deliberately.
- `icon-buffer 16 32 64 128` offers four sizes at once. Each is drawn in **its own colour**
  (16 red, 32 amber, 64 green, 128 blue), so the size the compositor chose is readable off
  whatever it renders — no logging needed.

Every buffer is drawn the same way: a fully transparent margin (alpha survives the trip), a
solid body, and a **half-alpha quadrant written premultiplied**, as `argb8888` requires. A
consumer that forgets to un-premultiply renders that quarter visibly darker than the body.

The overlay's `ICON` line reports the current request, whether the manager global was bound at
all, and the icon sizes the compositor advertised at bind time.

The subject overlay shows **buffer px · viewport destination · xdg configure** side by side,
plus output/​fractional scale, ack state, decoration mode and child counts, so divergences are
obvious at a glance.

## Reproducing the reported bugs

- **Decorations off when misbehaving** — `deco-ignore` + `deco-badsize`: watch the declared
  geometry frame vs the drawn chrome.
- **Buffer ≠ agreed size** — `buf-delta 40`, `buf-preack`, `geo-mismatch`; also the viewport
  path (`vp-dest` unlike both buffer and configure) and scale paths.
- **Popups / subsurfaces out of bounds** — `popup-move` / `sub-move` past the output edge,
  `popup-nest`, large negative `sub-move`.
- **Fractional / DPI scale** — `fs-*` / `dpi-*`: confirm layout follows logical size, not raw
  buffer px; `dpi-zero` / `vp-bad` provoke protocol errors to check clean handling.
- **Single-pixel renderer** — `sp-fill` (whole window becomes a solid color via a 1×1 buffer
  scaled by viewport) / `sp-sub`.

> Note: `vp-bad` and `dpi-zero` deliberately provoke `bad_value` protocol errors and will
> disconnect the subject — that is the point (it tests the compositor's error handling).

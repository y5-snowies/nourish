# y5 pen-stress

A standalone Wayland `zwp_tablet_v2` test client + visualiser for the y5
compositor's **pen / stylus** path. It binds the **whole** tablet-tool surface and
draws every parameter it receives, and can also run as a client that does **not**
bind the tablet protocol — so you can verify both the native stylus path and the
compositor's pointer-emulation fallback on the same surface. It is the pen sibling
of `developer.tool.touch/touch.stress`.

It is a **standalone crate** (its own `[workspace]` + `Cargo.lock`, crates.io deps
only) — deliberately outside the repo's `link.all.sh` graph and lint rules, exactly
like `touch.stress`.

## Build

```bash
cd compositor.developer/developer.tool/developer.tool.pen/pen.stress
cargo build --release
```

## Run

Point `WAYLAND_DISPLAY` at the target compositor (e.g. a nested y5) and launch:

```bash
./target/release/pen-stress                 # full: tablet + wl_pointer + wl_keyboard
./target/release/pen-stress --no-tablet     # DON'T bind the tablet → pointer-emulation path
./target/release/pen-stress --no-pointer    # pure zwp_tablet_v2 (no pointer)
```

The banner shows which path is active: **TABLET BOUND** (native `zwp_tablet_v2`) vs
**POINTER-ONLY** (the compositor is emulating a pointer because no tablet was bound).
If the compositor doesn't advertise `zwp_tablet_manager_v2` at all, that is reported
on stderr and the tool stays pointer-only.

## What it exercises (the more the merrier)

Every `zwp_tablet_tool_v2` axis and identity event:

- **tool type** — pen / **eraser** / brush / pencil / airbrush / finger / mouse / lens
  (colour-coded; eraser strokes *erase* the ink layer).
- **pressure** — nib radius + on-canvas ring scale with force; shown as `P nn%`.
- **distance** — hover gap; the hover ring shrinks as the tip nears the surface (`D nn%`).
- **tilt x / y** — drawn as a lean vector from the nib; shown as `TILT +x,+y`.
- **rotation** — barrel rotation tick; shown as `ROT`.
- **slider** — finger slider (`SLD`), and **wheel** degrees / clicks (`WHEEL`).
- **buttons** — per-tool stylus buttons, live set `BTN [..]`.
- **capabilities / hardware serial / wacom id** — from the tool's announce burst.
- **proximity in/out**, **tip down/up**, **frame** batching.

Plus the **tablet pad** (device body): buttons, ring angle, strip position — line
`PAD BTN [..] RING .. STRIP ..`.

Pressure-modulated **ink** is laid down while the tip is down (eraser erases), so a
whole stroke's pressure/tilt profile is visible at once.

## Keys (needs a keyboard)

- `t` — toggle the tablet seat live (destroys / re-creates it, so the compositor flips
  between native pen and pointer emulation without a restart).
- `c` — clear ink + log.
- `q` / `Esc` — quit.

## Scenarios to test

| Goal | How |
|---|---|
| Native pen works | default mode; hover then press — watch pressure ring + ink |
| Tilt / rotation reported | tilt/rotate the stylus, watch the lean vector + `TILT`/`ROT` |
| Eraser mode | flip the stylus to the eraser end — colour changes + strokes erase |
| Non-tablet app gets pointer | `--no-tablet`; the pen should move the pointer crosshair + click |
| Live switch mid-session | default mode, press `t` to drop the tablet and confirm the pointer path takes over |
| Pad controls | on hardware with a pad, press buttons / spin the ring / slide the strip |

# y5 touch-stress

A standalone Wayland `wl_touch` test client + visualiser for the y5 compositor's
touch path. It binds **every** `wl_touch` feature and draws what it receives, and
can also run as a client that does **not** bind touch — so you can verify both the
native multi-touch path and the compositor's pointer-emulation fallback on the same
surface. It doubles as a scaffold for developing future touch features (text
selection, etc.): the content area renders selectable text lines and every event is
logged on-screen.

It is a **standalone crate** (its own `[workspace]` + `Cargo.lock`, crates.io deps
only) — deliberately outside the repo's `link.all.sh` graph and lint rules, exactly
like `developer.tool.window/window.stress`.

## Build

```bash
cd compositor.developer/developer.tool/developer.tool.touch/touch.stress
cargo build --release
```

## Run

Point `WAYLAND_DISPLAY` at the target compositor (e.g. a nested y5) and launch:

```bash
./target/release/touch-stress                 # full: wl_touch + wl_pointer + wl_keyboard
./target/release/touch-stress --no-touch       # DON'T bind wl_touch → pointer-emulation path
./target/release/touch-stress --no-pointer     # pure wl_touch (no pointer)
```

The banner shows which path is active: **TOUCH BOUND** (native `wl_touch`) vs
**POINTER-ONLY** (the compositor is emulating a pointer because no `wl_touch` was
bound). This is the exact split the compositor makes via `client_has_touch` — a
touch is delivered natively only when the focused client bound `wl_touch`, else it
falls through to pointer emulation.

## What it shows

- **Per-finger** ring (sized by `wl_touch.shape` when sent), crosshair, slot `#id`
  (colour-coded), and a motion trail.
- **Pointer** crosshair (filled when a button is down) — this is what you see in
  `--no-touch` mode, driven by the compositor's emulation.
- **Event log** of every `down / up / motion / shape / orientation / cancel` (and
  pointer press/release), with slot ids and positions.
- A **content area** with text lines — the working surface for prototyping touch
  features (e.g. drag-to-select text).

## Keys (needs a keyboard)

- `t` — toggle `wl_touch` binding live (re-issues / releases `wl_touch`, so the
  compositor flips between native touch and pointer emulation without a restart).
- `c` — clear fingers + log.
- `q` / `Esc` — quit.

## Scenarios to test

| Goal | How |
|---|---|
| Native multi-touch works | default mode; place 2–5 fingers, drag, check slot ids + trails |
| Non-touch app gets pointer | `--no-touch`; a single finger should move the pointer crosshair + click |
| Live switch mid-session | default mode, press `t` to drop touch and confirm the pointer path takes over |
| Shape / orientation | on hardware that reports them, watch the ring size + SHAPE/ORIENT log lines |
| Gesture cancel | trigger a compositor gesture (e.g. 2-finger canvas zoom) → expect a `CANCEL` |

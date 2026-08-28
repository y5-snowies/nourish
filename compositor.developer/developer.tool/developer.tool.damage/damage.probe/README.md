# damage-probe

Isolates `wl_surface.damage` (surface-local) from `wl_surface.damage_buffer`
(buffer-local) under `wp_viewporter` and fractional scale — the configurations
where the two stop meaning the same thing.

```
make
WAYLAND_DISPLAY=wayland-2 ./damage-probe --damage=surface --src=50,40,200,150
```

## Why it exists

With no viewport and `buffer_scale == 1` the two entry points take identical
numbers, so a bug in either conversion is invisible. They diverge once a viewport
crops (`set_source`) or scales (`set_destination`) the surface, or the buffer
carries a scale — and y5 has both, plus fractional output scale.

## Method

Commit a **dark** buffer with full damage, let it reach the screen, then commit a
**bright** buffer reporting damage for exactly one rect through the chosen entry
point. A correct compositor repaints only that rect, so the bright region on
screen *is* the compositor's answer to "where did this damage land".

Deliberately `wl_shm`: damage semantics belong to the protocol, not to how the
buffer was allocated, and shm keeps GBM/EGL/driver variables out of it.

The 300ms settle between the two commits is load-bearing — committed inside one
compositor frame they collapse, the whole surface repaints, and the observation
is lost.

## Reading the result without a screen

The visual check needs eyes. For an automated read, run the compositor with
`Y5_DAMAGE_TRACE=1`, which prints, per commit, the surface view and each damage
rect with its converted buffer rect:

```
[Y5-DAMAGE] view src=(50,40 200x150) dst=800x600 off=(0,0) | buffer=400x300 buffer_scale=1 surface=400x300
[Y5-DAMAGE]   damage        in=(100,80 120x60) -> buffer (75,60 30x15)
```

That trace lives at the single point where smithay reconciles the two entry
points (`vendor/smithay/src/backend/renderer/utils/wayland.rs`, in
`RendererSurfaceState::update_buffer`): `damage_buffer` is already in buffer
space, `damage` goes through `surface_view.rect_to_local().to_i32_up().to_buffer()`.
It is `eprintln!` rather than y5's log macros on purpose — a probe must not
depend on a subscriber being installed.

## Options

| flag | meaning |
|---|---|
| `--buffer=WxH` | buffer size in pixels (default `400x300`) |
| `--buffer-scale=N` | `wl_surface.set_buffer_scale` |
| `--src=X,Y,W,H` | `wp_viewport.set_source`, **buffer** coords |
| `--dst=WxH` | `wp_viewport.set_destination`, **surface** coords |
| `--rect=X,Y,W,H` | the damage rect, in whichever space `--damage` selects |
| `--damage=surface\|buffer` | which entry point (default `surface`) |
| `--hold=MS` | ms to stay up after the damaged commit |

## Results — y5 nested (winit), 2026-08-20

Buffer `400x300`, damage rect `100,80 120x60`, output fractional scale 1.25×.

| case | `damage` (surface) → buffer | `damage_buffer` → buffer |
|---|---|---|
| plain | `100,80 120x60` | `100,80 120x60` |
| `buffer_scale=2` | `200,160 200x120` | `100,80 120x60` |
| `src=50,40 200x150` | `150,120 120x60` | `100,80 120x60` |
| `dst=800x600` (2× up) | `50,40 60x30` | `100,80 120x60` |
| `src=50,40 200x150` + `dst=800x600` | `75,60 30x15` | `100,80 120x60` |
| `dst=300x225` (1.333× down) | `133,106 161x81` | — |

**All correct.** Checking the two that could not be guessed:

- **src + dst.** Surface coords live in the dst space (800×600) and map onto the
  src region (200×150 at 50,40), so the factor is `200/800 = 0.25`:
  `(100,80 120x60) × 0.25 = (25,20 30x15)`, plus the src origin `(50,40)` =
  **`(75,60 30x15)`**. Exactly what came out.
- **Fractional (1.333×).** `100 × 4/3 = 133.33`, `80 × 4/3 = 106.67`, and the far
  edges `220 × 4/3 = 293.33`, `140 × 4/3 = 186.67`. `to_i32_up()` rounds the rect
  **outward** — origin down, far edge up — giving `133,106` with size
  `294-133 = 161` and `187-106 = 81`. Outward is the safe direction: over-damage
  costs a repaint, under-damage leaves artifacts.

`damage_buffer` is invariant across every row, which is the point — buffer
coordinates do not depend on the viewport.

### One observation, not a defect

With `src=50,40 200x150` the surface is 200×150, so a `100,80 120x60` surface
rect runs past the surface's right edge. smithay clips the converted rect to the
**buffer** (`.intersection(buffer_dimensions)`) but not to the surface, so the
result keeps its full width. The effect is over-damage, which is safe; it is
recorded here so a future reader does not re-derive it as a bug.

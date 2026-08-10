# Background shader examples

Example bundles for the runtime parallax-background shader pipeline. Each is a
folder under `<name>/` containing one or more **format subfolders**; the loader
auto-detects the format and, for the active renderer, tries the available
formats in that renderer's preferred order, falling back to the built-in
parallax if none compile.

| Example | Folder(s) | Vulkan | GLES | Exercises |
|---|---|---|---|---|
| `aurora` | `wgsl/` | WGSL → SPIR-V ✓ | falls back to built-in | single-source WGSL |
| `plasma` | `glsl/` | GLSL(450) → SPIR-V ✓ | falls back to built-in | single-source desktop GLSL + generated vertex |
| `dual` | `vulkan/` + `gles/` | WGSL → SPIR-V ✓ | raw ES-1.00 ✓ | explicit per-backend authoring |
| `ripple` | `gles/` | falls back to built-in | raw ES-1.00 ✓ | native-GLES-only + per-renderer fallback |
| `broken-wgsl` | `wgsl/` | compile error → built-in | built-in | WGSL compile-error fallback |
| `broken-glsl` | `glsl/` | compile error → built-in | built-in | GLSL compile-error fallback |

## The `Multipass` set — `mp-*`

**These bundles no longer live here.** They ship compiled INTO the binary, from
`compositor.expansion/compositor.background/background.two/two.shader/shader.embed/bundles/`
— `include_str!` makes them a build input, and shipped source belongs beside the
code that ships it rather than in a documentation tree a packager may drop. They
need no `install-shaders.sh` run and appear in the picker on a fresh install.
The table below is kept here because this is where the shipped set is described.

Everything else in this folder exists to exercise ONE mechanism, is named after
the mechanism, and looks like a test. The `mp-*` bundles are the opposite: things
to actually run. They all draw the **stock parallax** as their background, from
one shared `parallax_scene` in `mp-parallax/lib/parallax.wgsl`, and they all
declare `"category": "Multipass"` so they group together in the picker instead of
being lost among sixty `tb-*` folders. A bundle you drop into the shader folder
yourself is grouped separately even if it declares that same heading — its
heading carries a person icon (see `shader.builtin::USER_MARK`).

Sharing the background is the point. Switching between them changes the effect and
nothing else, and because the per-world overrides are keyed BY NAME the three
background knobs (`drift`, `stars`, `nebula`) keep their values across the switch.

| Bundle | What it is | Asks the engine for |
|---|---|---|
| `mp-parallax` | The background alone — the baseline the rest are a delta from, and the file the others `#import`. | *(nothing)* |
| `mp-effects` | Eight knobs, each off at 0: frost, glass, transparency, key tolerance, sticky edge, motion blur, light motes, vignette. The one that exercises nearly the whole surface. | `world_geometry`, `world_textures`, `composited_scene`, `previous_frame` |
| `mp-crt` | A tube over the whole desktop: barrel warp with a **pointer corrected through the same function**, scanlines, aperture grille, fringing, phosphor glow. Aspect-normalised *and* overscanned, so `curve` means the same bend at 16:9 and at 32:9 and the picture fills the screen with no black surround. | `composited_scene` |
| `mp-colorblind` | Simulate or correct for protan / deutan / tritan vision, in linear light. | `composited_scene` |
| `mp-grayscale` | Desaturate, by Rec.709 luma or flat average. | `composited_scene` |
| `mp-invert` | White ↔ black, either a full negative or lightness-only (which keeps hues readable). | `composited_scene` |
| `mp-levels` | Exposure (stops) → contrast (pivoted at mid grey) → brightness, **twice**: once for everything, once again for window rectangles only. Every default is the identity. | `world_geometry`, `composited_scene` |

**`mp-effects` is the one worth reading.** Its three window effects all answer the
same question — *what is behind this window?* — which no engine-provided image
contains: `content` is the desktop with the windows already composited in, and
behind a window is usually another window rather than the wallpaper. So the pass
rebuilds the band itself from the world texture array, back to front, over the
background it drew; the accumulation just before the front-most drawable is
exactly what that drawable sits on. Frost blurs it, glass bends it through a lens
that grows toward the window's edges, and transparency composites the window back
over it with `tb-window-chroma`'s auto chroma key so the window's flat chrome
drops out and its text does not.

That is why it is the only bundle in the set that needs the device's
**descriptor-indexing** feature. Everything else loads anywhere the Vulkan
renderer does.

All seven set `decorations: off` and `letterbox: on-resize`: the engine's border
is drawn outside the window's slot and is opaque, so any effect that reads window
edges reads the border instead of the window, and the letterbox bars would sit
inside the rect that `behind()` is compositing.

All are `place: auto` (the default), so the worker takes what it can and the rest
runs inline — nothing here asks for a placement it might not get.

## Multipass bundles (`pipeline.json`)

A bundle may instead ship a **`pipeline.json`** manifest describing a *graph* of
passes with named intermediate targets and `#import`-composed modules (Vulkan
only; GLES falls back to the built-in). See `document/SHADER_PIPELINE.md`.

| Example | Format | Vulkan | GLES | Exercises |
|---|---|---|---|---|
| `bloom` | `pipeline.json` | 5-pass ✓ | falls back to built-in | intermediate targets, ½-res ping-pong blur, naga_oil `#import` + shader-def variants |
| `vignette` | `pipeline.json` | after-content ✓ | falls back to built-in | an `after-content` pass sampling the composited `content` (background + windows) — darkens edges ABOVE the windows |
| `window-glow` | `pipeline.json` | after-content + window-rects ✓ | falls back to built-in | `needs: ["window-rects"]` — reads the on-screen window rectangles (a `@group(1)` UBO) and paints a soft bright halo around each window |
| `motion-blur` | `pipeline.json` | after-content + history ✓ | falls back to built-in | a pan-linked `backdrop` (before) + an after-content `blur` that disk-blurs the built-in `history` target (previous frame's world content) and blends it over `content`, radius/weight ∝ pan speed (magnitude of the packed velocity in the `lock_alpha.w` engine lane) — a drastic pan-gated smear (incl. the background), no-op when the camera is still |
| `glass` | `pipeline.json` | after-content + window-rects + window-textures + history ✓ | falls back to built-in | req-6 frosted glass, authored purely as a shader: `needs: ["window-rects","window-textures"]` gives per-window rectangles + a bindless `binding_array` of window textures; per pixel it finds the front-most covering window, samples that window's texture (its premultiplied alpha is the mask) and composites it over a disk-blurred `history` backdrop — opaque text stays crisp, translucent fills frost. Requires the device's **descriptor-indexing** feature (probed at init); falls back where absent. Shader must `enable wgpu_binding_array;` |

`bloom`: `base → scene`, `bright → bright(½)`, `blurH/blurV` (one `blur.wgsl`,
`HORIZONTAL` shader-def toggled) `→ blur_a/blur_b`, `combine → output`. Two
`@prop` sliders (`threshold`, `intensity`) drive it live. Every pass compiles
through the runtime naga_oil path (unit-verified).

Notes:
- The Vulkan backend is the primary path (`compositor_prefers_dmabuf`). `wgsl/`
  and `glsl/` are Vulkan-only here: emitting GLES from naga needs an ES-3.00
  smithay path we deliberately don't add, so on GLES they fall back to built-in.
- Vulkan shaders use the standard 48-byte engine `Push`
  (`res_zoom_time` / `pan_flow` / `lock_alpha`); GLES shaders declare the engine
  uniforms they use (`u_time`, `u_resolution`, …) and write `gl_FragColor`.
- `// @prop name kind k=v …` lines declare tunable variables. Each prop maps to
  a fixed param slot (prop #i → `u_param0/1` on GLES, the push `params` block on
  Vulkan); the shader must actually read its slot to react (the examples here
  do). The settings **Current World** tab renders a slider/toggle per prop,
  edits drive the live background and persist per-world, and a live wgpu preview
  (drag to pan, scroll to zoom) sits at the top.

## Install

Copy into the user data dir the loader scans:

```sh
mkdir -p ~/.local/share/y5/background/shader
cp -r document/shader-examples/* ~/.local/share/y5/background/shader/
```

Then pick one per world from **Settings → Current world → Background shader**, or
set the new-world default in `~/.config/y5.compositor/preferences.json`:

```json
{ "background_shader": "aurora" }
```

---

## Triple-buffering stress set (`tb-*`)

Ten multipass bundles built to exercise the failure modes that matter when the
background runs on the **off-thread worker** (`two.worker`, "background triple
buffering"). They are ordinary bundles — nothing here is worker-specific — but
each isolates one property that decides whether a graph *can* be offloaded, or
that would misbehave once it is.

**Offload class** is the key column. The worker owns a separate `VkDevice` and
has no window set, so a pass is worker-eligible only if it crosses no boundary:
`before-content`, no `needs`, and no post-composite input. `WHOLE` = the entire
graph qualifies. `BEFORE-BAND` = only the leading band does; the rest must stay
in the compositor.

| Bundle | Passes | Offload class | Stresses | Watch for |
|---|---|---|---|---|
| `tb-heavy-single` | 1 | WHOLE | raw GPU cost, one heavy pass | biggest expected TB win; input latency while panning if it is NOT offloaded |
| `tb-chain-6` | 6 | WHOLE | pass *count* / record cost, mixed ½ and ¼ scales | per-pass pipeline build churn on first select |
| `tb-wide-32f` | 3 | WHOLE | bandwidth: two **full-res `rgba32f`** intermediates (~132 MB/pane at 4K) | allocation cliff — worker holds a set *per slot*; refused allocation gates the pane |
| `tb-after-trivial` | 2 | BEFORE-BAND | the fixed cost of the after-content path | overhead only — cheapest possible after pass, so any delta is the offscreen `content` image + forced full-frame redraw |
| `tb-after-history` | 2 | BEFORE-BAND | `history` + the offscreen gate | **diagnostic**: red ghost = `content − history`. Ghost width is how far history lags — if it widens or pulses under TB, the history contract has drifted |
| `tb-window-rects` | 2 | BEFORE-BAND | `needs: ["window-rects"]` — numbers only | outline must track windows exactly while dragging; lag = clock gap between bands |
| `tb-window-tex` | 2 | BEFORE-BAND | `needs: ["window-textures"]` — bindless client dmabufs | hardest to offload; also the descriptor-indexing fallback (expect stock background + warning on devices without it). Wrong colours on the wrong window = index misalignment |
| `tb-velocity-probe` | 2 | BEFORE-BAND | the `lock_alpha.w` velocity lane **and** clock divergence | **diagnostic**: cyan = speed, yellow = `fract(time)`; the before band draws the top pair, the after band the bottom pair. Bars separating = the bands are on different frames |
| `tb-many-targets` | 9 | WHOLE | 8 targets across 4 scales and 3 formats | resize / monitor hotplug while running: a leaked or un-retried allocation shows up as a background that never returns |
| `tb-mixed-scale` | 3 | WHOLE | downsample → upsample round trip on a 1px grid | right half must be a *centred* blur of the left; a shift means half-texel error in scaled-target sampling |

### Start here

- **Did the multipass chain actually run?** `tb-chain-proof`. Three lights: all
  green means every pass ran, in order, reading the right target. This is the
  bundle to trust, and the one to use for an offload A/B — `bloom` is NOT a
  verification bundle. Bloom looks broadly the same whether its five passes ran or
  a single-pass fallback did, so it can only ever tell you something rendered, not
  that the graph is correct. Same trap as judging window textures by "a window
  appeared".
- **Is the velocity lane alive?** `tb-velocity-probe` — pan the world, the cyan
  bar must grow. If it stays flat, every velocity-driven bundle (`motion-blur`)
  is silently dead. This is the bundle to check first after any change to the
  engine push ABI.
- **What does after-content actually cost?** `tb-after-trivial` vs
  `tb-heavy-single`. The former is a trivial shader on the expensive path; the
  latter an expensive shader on the cheap path.
- **Will this graph ever offload?** Read the class column. `WHOLE` bundles are
  the ones triple buffering can help today.

### ABI note

These all decode the pan velocity as two snorm16 halves:

```wgsl
let vel = unpack2x16snorm(bitcast<u32>(pc.lock_alpha.w)) * 16384.0;
```

A bundle written against the older *scalar* `lock_alpha.w` reads that packed word
as a float — a denormal near zero — and silently computes a speed of ~0. If a
velocity-driven bundle stops responding, check this line first, and re-install
the bundle from `document/shader-examples/`.

### Loud window bundles (added after first hardware pass)

The first `tb-window-*` pair was too subtle, and `tb-window-tex` had a diagnostic
bug: it blended by the sampled alpha, so an unbound texture array produced alpha
0 and the pass rendered *nothing* — visually identical to the bundle never
loading. These replace/extend it. **None of them gate their output on the data
being tested.**

| Bundle | Needs | What you should see |
|---|---|---|
| `tb-window-spotlight` | `window-rects` only | Everything outside windows crushed to near-black; inside boosted with animated diagonal stripes and a thick amber border. No windows open → dark screen + red corner marker. **Cannot silently fall back** — rects need no device capability |
| `tb-window-mirror` | rects + textures | Each window replaced by its own texture, **mirrored horizontally** — text reads backwards. Dead array → flat red body, never a no-op |
| `tb-window-xray` | rects + textures | Each window as a glowing cyan wireframe (Sobel over 8 bindless taps) on a darkened body — windows become blueprints. Exercises *multi-tap* client-dmabuf reads |
| `tb-window-tex` (rewritten) | rects + textures | Three independent signals: magenta **beacon** (pass ran) · green **count blocks** (rects arrived) · bottom **thumbnail wall** drawn opaque, with alpha as a separate swatch (textures bound). Seeing none of the three = the bundle did not load |

**If a `window-textures` bundle shows the stock background**, `load_multipass`
gated it: the device lacks descriptor indexing, so the whole bundle fell back to
the single-pass path. It logs a `warn!`, and the device probe logs
`descriptor_indexing=` at renderer init — check that first.
`tb-window-spotlight` is the control: it needs no such capability, so if it works
and the texture bundles don't, the cause is device capability, not the pipeline.

### `tb-parallax-lights` — the realistic variant, and the clock-coherence test

A real parallax background (three star layers drifting against the world pan)
carrying three moving **emitter** artifacts, plus an after-content pass that
lights the **windows** from those same emitters. Windows are treated as flat
panels with a soft bevelled edge, so a light to the left of a window rims its
left edge; there is a broad proximity wash and a Blinn specular sheen that sweeps
as the emitter moves. Additive, so window content stays readable.

It is the only bundle where **both bands must agree on shared geometry**. Neither
pass is told where the lights are — `lib/lights.wgsl` is `#import`ed by both, and
each derives `light_pos(i, time)` from its own push. (It is also the second user
of the `modules` / naga_oil `#import` feature after `bloom`.)

That makes it the acceptance test for clock coherence: if the before band is ever
paced by the worker while the `lit` pass runs in the compositor, the two clocks
diverge and **the rim highlight points at a place the glowing orb isn't**. Sight
along the line from an orb to the window edge it should be lighting — if they
disagree, offload has broken the shared-clock assumption. Every other bundle
would show that as a cosmetic wobble; here it is a visible correctness bug.

It doubles as a window-rect lag test: the lighting is anchored to the rects, so a
rect trailing its window shows up as light sliding off the panel as you drag it.

### `tb-window-owned` — pipeline-owned window compositing (§8d)

The demonstrator for `"windows": "pipeline"`. The engine stops blitting client
windows into the world band and publishes only the window set; this bundle's
single before-content pass draws the background **and** composites every window
itself — each one gently **rotated** and **translucent** over an unbroken grid.

Both of those are impossible for every other bundle here. Once the engine has
flattened a window into `content`, the background behind it is gone: you can
paint over a window, but you cannot move it, tilt it, or see through it. This is
the bundle that proves the difference.

Reading it:

| What you see | Meaning |
|---|---|
| Rotated, see-through windows over an intact grid | working |
| Windows missing entirely | the pass is not drawing them — check `descriptor_indexing=` in the log |
| Windows upright and opaque | the engine is still blitting them; `windows: pipeline` did not take effect |

It needs no offscreen path at all — the whole world band is produced inside the
graph, which is exactly the shape that becomes worker-offloadable once
client-dmabuf import lands. Today it declares `needs`, so its placement verdict
is correctly `None`.

### `tb-window-none` — the negative control for §8d

Claims full window-compositing responsibility (`"windows": "pipeline"`) and then
draws **no windows at all**. The desktop should show its diagonal sweep and
nothing else, however many clients are open.

Any window pixel on screen means something is still blitting windows behind the
pipeline's back — a path `skip_windows` does not cover. It isolates *did the
engine really stop drawing* from *did the pipeline draw them correctly*, which
every other window bundle conflates.

**Read the top-left blocks first** — one white block per window in
`windows.count`, drawn from the rects UBO, not from window content. A bundle that
silently failed to load would also show a bare desktop, so a blank screen alone
proves nothing:

| Blocks | Window content | Verdict |
|---|---|---|
| present | none | **PASS** — suppression works |
| present | visible | **MIS-BLIT** — something else drew them |
| absent | — | bundle never loaded; not a valid run (check the log) |

It needs only `window-rects`, so it does not depend on descriptor indexing and
runs where the bindless bundles fall back. It is also a regression test for the
input-less window pass that used to build a pipeline layout with no set 0.

### `tb-window-chroma` — per-window auto chroma key (needs §8d)

Estimates each window's **dominant background colour** and punches every pixel
within ±tolerance of it out to transparent, so the wallpaper shows through the
window's chrome while its text and content stay solid.

This bundle is impossible without `"windows": "pipeline"`. Making a window pixel
transparent needs something behind it to reveal, and the engine's flattened
`content` has already overdrawn the background — so it owns window compositing and
draws them over a background it still has.

The estimate is 9 probes in a 3×3 grid at 8% / 50% / 92% of the window: corners
and edge midpoints land on chrome (what we want to key), the centre probe stops a
full-bleed window keying its own content. The "dominant" colour is the probe with
the most neighbours inside `tol`, so gradient chrome resolves to its dominant band
rather than an average nobody actually has. Props: `tol`, `soft` (feathers glyph
edges so they don't fringe), `keep` (residual alpha — set >0 to tint rather than
fully key).

It is deliberately brute force: 9 probes and 81 comparisons per pixel, recomputed
for every pixel of every window even though the answer is constant per window. A
reduction pass would need a per-window target, which output-fraction targets
cannot express. That redundancy is realistic for "expensive per-window analysis"
and is exactly what a stress bundle should cost.

### `tb-window-descriptors` — every descriptor and timestamp at once

A test card for `window_geometry` + `window_textures` + `window_times`. It owns
the whole world band so it can draw each window itself, and every visual choice is
picked to make ONE descriptor unmistakable rather than to look good — if a value
is wrong you should be able to say which without reading the shader.

| what | reads | look for |
|---|---|---|
| `ACTIVATED` | flag | the unfocused windows grey out and dim |
| `TOPMOST` | flag | a bright outline, on exactly one window |
| `FULLSCREEN` | flag | that outline goes solid instead of dashed |
| `RESIZING` | flag | the window shears with a scanline wobble |
| `MOVING` | flag | the window leans, top-to-bottom, while dragged |
| opened | time | a new window grows and fades in over ~0.6 s |
| entered / left | time | returning flashes a white rim — **blue** if it was gone > 3 s |
| focused | time | a ring sweeps inward from the border, ~0.5 s |
| topmost | time | the outline pulses once on becoming front-most |
| resize started / ended | time | drives the wobble; green settle-flash when it stops |
| move started / ended | time | eases the lean in; amber settle-flash on drop |
| pointer position | `pointer_state` | a crosshair follows the cursor, over everything |
| pointer buttons | `pointer_state` | the crosshair fills — white left, red right, green middle |
| last pressed / released | `pointer_state` | a ring **expands** on press, a thinner one **contracts** on release |

The six props are gains, not switches: set one to 0 to take an effect out of the
picture while checking another.

The two gestures are deliberately given *different* effects — a wobble for resize,
a steady lean for move — and different settle colours, so a lane swap between them
would be obvious rather than plausible. The two cursor rings differ in *direction*
for the same reason: press expands, release contracts, so you never have to catch
which one fired.

Two things it demonstrates about the ABI rather than about itself. The timestamps
are **absolute moments**, so every age here is `t - times.…` computed in the
shader — which is what makes the animations advance per frame instead of per
publish. And `left` is only ever read *on re-entry*: an off-screen window has no
entry in the array, so the blue rim is the only way that value can be observed at
all.

### `tb-chain-bleed` — the graph, with a way to see it

The replacement for reading `bloom`. Ten passes, six targets at three scales, and
**four** WGSL files — because `blur.wgsl` is compiled six times (three levels ×
two axes) via `"defines": { "HORIZONTAL": "true" }`, and `down.wgsl` twice.

A multi-scale edge bleed over the composited desktop: window borders, text and
panel edges glow, with separate **Tight / Medium / Wide** weights so the *shape*
of the glow is adjustable and not just its amount.

**The `Show` dropdown is the point.** Set it to Level 1, 2 or 3 and the screen
becomes that rung of the pyramid alone, full-screen and unweighted — three
visibly different blur extents. A chain of targets you cannot inspect is one you
have to take on trust; one `int` prop with `choices` fixes that.

Also the only bundle exercising, in one place:

| | |
|---|---|
| `defines` | one source, six passes — the separable-blur idiom |
| ping-pong | each level owns `_a`/`_b`; a pass cannot read what it writes |
| target `scale` | 0.5 / 0.25 / 0.125, so each level costs a quarter of the last |
| `cadence` | the coarsest level runs every 2nd frame — **all three of its passes**, see below |
| `int` + `choices` | the `Show` selector |
| `bool` | `Edges, not brightness` in `extract` |

Turn `Edges` **off** to see why it is on: a plain brightness bloom on a dark
desktop is the "nothing appears to happen" that makes a chain impossible to
believe in. On, the extract is a local-contrast test, so it fires on any
wallpaper.

**Cadence covers a sub-chain, not a link.** The first version throttled only the
level-3 blurs and left its downsample running every frame, so that target
alternated between raw and blurred — a per-frame flicker in the widest halo,
scaling with the `Wide` weight, which reads as the effect being unstable rather
than as the graph being wrong. The engine now refuses a target written at two
different rates, naming both passes.

**A caveat the file records:** the `@prop` parser accepts `float`, `int`, `bool`,
`vec2/3/4` and `color`, but the params block reserves **one float slot per prop**,
so a multi-component value only ever delivers its first lane. Use `float`, `int`
and `bool`; the rest parse and then quietly disappoint.

### `tb-persist-trail` — a target that survives the frame

Exercises `"persist": true`. Drag a window: it leaves a fading trail of its own
pixels along the path it actually took, curves included — which is what separates
a real accumulator from a directional smear.

Two passes, and the split is the lesson. `accumulate` names `trail` as both its
input and its output, which is legal *only* because a persistent target is a pair
of images the engine alternates: the read is last frame's, the write is this
frame's. `present` then draws the desktop over the ghost, and owns the world band
so the windows land on top — otherwise the trail would only be visible outside
every window, which is mostly where a trail is, so the bug would look nearly right.

**Set `decay` to 1.0** and nothing ever fades: the screen fills with every position
the window has occupied since you selected the bundle. That is the clearest proof
the target is genuinely persisting rather than being re-cleared.

Needs no device feature — it runs anywhere the Vulkan renderer does.

### `tb-storage-embers` — 4096 particles that live on the GPU

The effect the storage feature exists for. Glowing motes drift across the desktop,
pull toward the cursor, scatter when you click, and are pushed out of any window
they touch — so they pour around the edges of your windows like sparks around a
stone.

It needs storage for **two** separate reasons, and the second is the interesting
one:

1. A particle has state that outlives the frame — position and velocity — and it
   is not attached to a pixel. `parts[i]` is written by invocation `i`, whatever
   pixel that invocation happens to be shading. A render target can only ever
   write the pixel being shaded, so it cannot hold a particle at all.
2. Each ember then **splats** its light into a coarse grid at the cell its own
   position selects — the destination index is computed from a value, many embers
   land in the same cell and add up, and most cells get nothing. That is scatter
   in the strict sense.

The grid is what makes it cheap. Without it, rendering means looping over 4096
embers per pixel, two million times; with it, the simulation does a handful of
atomic adds per ember and the render pass does one bilinear read.

**The splat is spread, not a point, and that is what keeps the grid invisible.**
The first version dropped each ember's whole contribution into the one cell
containing it, and the grid was plainly visible — light jumped from cell to cell
as an ember moved, and every mote was a hard-edged square. That was not a
consequence of the field being a buffer, which is the obvious suspect and the
wrong one: a nearest-cell splat quantises the light whatever the field is stored
in, and the identical code writing to a storage *image* would look the same,
because filtering happens on the read and the damage is done on the write. The fix
is on the write — each ember distributes over a 3×3 neighbourhood with normalised
weights, so moving it a tenth of a cell changes the field by a tenth of a cell's
worth. The read is then plain bilinear, which is exactly what an image would do in
fixed-function hardware; what an image buys here is the ALU, not the smoothness.

Three passes for the same ordering reason as the histogram: `clear` zeroes the
per-frame light field, `simulate` moves the embers and splats, `render` reads the
field. Note that the two buffers want *opposite* things from persistence — the
field must be emptied every frame, the particles must not — which is why it
declares two rather than one.

### `tb-storage-histogram` — a buffer written at arbitrary indices

Exercises `storage`. A live luminance histogram of the desktop along the bottom of
the screen: open a dark window and the left bars grow, a white one and the right
bars grow.

This is the *smallest* honest use of scatter — `tb-storage-embers` above is the
one that is actually an effect; this one is the minimal probe, useful when
something is wrong and you want to know whether atomics and barriers work at all. Every pixel adds to the one bin its own
brightness selects, so a pixel at the top of the screen writes to a word a pixel at
the bottom also writes to — something a render target cannot express, because
rendering only ever writes the pixel being shaded.

**Three passes on purpose.** `reset` zeroes the bins, `tally` scatters into them,
`graph` reads them back. Doing all three in one pass does not work: invocations
within a single draw have no ordering relative to each other, so some pixels would
add before the clear and some after. It would still *look* like a histogram — just
a different arbitrary subset of the pixels every frame — which is exactly why this
is worth a test bundle. The engine's storage barrier between intermediate passes is
what makes the three-pass version correct.

`atomicAdd`, not `+= 1`, for the same class of reason. The bars are coloured by
which bin they are, so the graph reads left-to-right as dark-to-light and a
mis-indexed bin shows up as a colour out of order.

Requires the device's **`fragmentStoresAndAtomics`** feature; the bundle is refused
outright where it is absent rather than silently having its writes discarded.

### The `tb-texture-*` set — the one input a shader cannot compute

Exercises `textures`. Everything else the engine offers is either generated
(noise, gradients, SDFs) or is the desktop itself; a texture is where authored
detail comes from — art, a measured table, a scanned surface.

Wiring is deliberately not a new mechanism. A bundle declares a name under
`textures` in `pipeline.json`, a pass names it in `inputs` exactly as it would
name a target, and it lands at `@group(0) @binding(1 + n)` in the same
sorted-by-binding-name order as every other input. A shader cannot tell whether
what it is sampling is a PNG or an intermediate the graph rendered, and nothing
downstream carries a second list. No new bind group, no push field, no per-frame
engine work: the whole cost is one decode and one upload, at load.

The assets are generated by **`make-texture-assets.py`**, committed beside the
bundles, so the art is reviewable as code rather than as an opaque binary — and
so a change to it is a diff.

**`tb-texture-all` is the covering example** — start there. Three textures in one
bundle, all three colour-space roles, a texture and an engine built-in mixed in one
`inputs` map (so the sorted binding order is visible: `lut` = 1, `scene` = 2,
`sparks` = 3), the same atlas read by two passes in two different *bands*, and a
bundle-wide `Show` dropdown declared in both passes — props union by name, so that
is one slider and one value, which is how a debug mode spans a graph. It carries
its own copies of the three PNGs rather than pointing at its siblings, because a
texture path may not contain `..`; the generator writes them.

The other three are the same material isolated one idiom at a time:

- **`tb-texture-sheet`** — art. A 4×4 sprite atlas of a spark igniting and
  dying, orbiting every window, each spark on its own phase. Sixteen frames of
  drawing that no expression here would produce. Also the two traps: the sampler
  clamps at the edge of the *whole atlas*, not per cell, so the shader insets by
  half a texel (`textureDimensions` is how it learns the size — the push does not
  carry it); and a sampled texel is **straight** alpha, so compositing is
  `rgb * a`, done in the shader, in linear space.
- **`tb-texture-grade`** — data. A 16×16×16 colour LUT as a horizontal strip,
  applied after-content so the grade covers windows too. Declared
  **`"srgb": false`**, and that is the whole point of the example: those bytes
  are coordinates, not colours, and linearising them corrupts every entry while
  still rendering something. Flip it to `true` and reload to see what a wrong
  colour space looks like — it reads as a bad grade, not as a bug, which is why
  the flag is declared per texture and never guessed.
- **`tb-texture-paper`** — a material. A seamless 256² tile storing a height
  field in R and its two slopes in G/B, lit by a light that follows the pointer.
  Tiling without a repeat sampler: there is one sampler and it clamps, so the
  wrap is `fract()` in the shader, which is legal only because the image is
  authored seamless. Sample it with `textureSampleLevel(..., 0.0)` — `fract` is
  discontinuous at every tile edge and an automatic-LOD sample reads that jump as
  a huge derivative and bands along every seam.

**Caps, all refused rather than trimmed** (the storage rule, for the storage
reason — a bundle handed less than it declared reads the wrong thing everywhere
and cannot tell):

| Cap | Value | Where |
|---|---|---|
| Longest edge, per image | 4096 | checked from the file **header**, before the decoder allocates |
| Decoded bytes, per bundle | 64 MiB | `Σ w·h·4` — never file size, which is off by orders of magnitude in the attacker's favour |
| Images bound by one pass | 12 | targets **and** textures — they share `@group(0)` bindings, and Vulkan's guaranteed floor is 16 |

A texture path may not escape its bundle (no `..`, no absolute), unlike
`modules`, which share sources between sibling bundles on purpose. PNG only:
`image`'s JPEG decoder does not build in this configuration, and JPEG is lossy
and alpha-less — strictly worse for sprites, unusable for LUTs and masks.

Editing a texture and calling `Shader::reload` works, because the file's bytes
are part of the pipeline's content hash. They have to be: left out, the id would
not move, `GraphExec::prepare` would see no change, and the repaint would never
reach the GPU. Same failure `tests/reload.rs` exists for, one input later.

### `tb-crt` — old-television tube (whole-picture after-content)

Same family as `vignette` — one after-content pass over the composited scene —
but where vignette only *attenuates* what is already there, this **resamples the
whole picture** through a curved screen. Barrel curvature → bezel cutoff → RGB
channel separation → aperture-grille mask → scanlines → rolling refresh bar →
vignette → bloom → mains flicker, over an SMPTE-ish colour-bar backdrop.

Why it stresses a different axis from every other bundle:

- every output pixel reads `content` at a **different** place, so nothing is a 1:1
  blit and the pass cannot be meaningfully damage-scissored — the honest worst
  case for the after-content path;
- three taps per pixel for chromatic aberration make it **bandwidth**-bound on a
  full-res target rather than ALU-bound;
- the tube mask is computed at output-pixel frequency, which is precisely where
  fractional scaling and non-native resolutions produce moiré — a visual canary
  for the scaled-target sampling `tb-mixed-scale` only tests structurally.

### `-inline` twins — A/B the same scene with and without offload

Six bundles are fully offloadable (`Offload::Whole`): every pass is
`before-content`, declares no engine `needs`, and the band writes `output`. With
triple buffering on, those run on the background worker. Each has a twin:

| Offloaded | Inline twin |
|---|---|
| `bloom` · `tb-chain-6` · `tb-heavy-single` | `bloom-inline` · `tb-chain-6-inline` · `tb-heavy-single-inline` |
| `tb-many-targets` · `tb-mixed-scale` · `tb-wide-32f` | `tb-many-targets-inline` · `tb-mixed-scale-inline` · `tb-wide-32f-inline` |

A twin is the **same graph** with `"place": "compositor"` on every pass, which
pins it to `Offload::None` and keeps it in the compositor's command buffer.
Switch between the pair and the picture is identical; only the path changes. That
is the comparison worth making — frame rate, and input latency while panning.

The twin's `shader` paths point at the ORIGINAL bundle (`../<name>/passes/...`)
rather than copying the sources. A copied twin could drift, and a drifted twin
silently invalidates the very comparison it exists for. `shader.pipeline`'s
`shipped` test loads both and asserts their pass and target counts still match.

**Every bundle now has a twin** — all 24 — but only the six above are a live A/B
today. For the other eighteen the original is not offloadable yet, so original and
twin both run inline and are indistinguishable. They exist so the pairs are
already in place when the later stages land: once an after-content band can run
on the worker (stage 3-on-worker / stage 5), the ORIGINAL starts offloading while
its `-inline` twin stays pinned, and the comparison becomes live with no new
bundles to write and no risk of authoring drift in between.

Until then, treat a non-offloadable pair as one scene listed twice.

| Live A/B today | Pre-built for later |
|---|---|
| `bloom`, `tb-chain-6`, `tb-heavy-single`, `tb-many-targets`, `tb-mixed-scale`, `tb-wide-32f` | the remaining 18, incl. `glass`, `tb-crt`, `tb-parallax-lights`, `tb-window-*` |

Why they cannot offload yet: an after-content pass needs the composited `content`,
and a `window-*` pass needs the window set — neither of which the worker has.

### `tb-cadence` — per-pass frame division

`"cadence": N` on a pass runs it only every Nth frame; between runs it is simply
not recorded and its target keeps the previous result. The intermediates are
allocated once rather than per frame, so "hold" costs no extra storage and no
copy — skipping the record *is* the hold.

The bundle draws two arms on one dial: cyan from the `output` pass at full rate,
orange from a `cadence: 8` intermediate.

| What you see | Meaning |
|---|---|
| cyan sweeps smoothly, orange jumps in steps | cadence working |
| both smooth | cadence ignored |
| orange absent | the held target is being cleared between runs |

Cadence is rejected on the `output` pass (`plan()` errors): skipping that would
leave the frame with **no** picture, which is a different thing from a stale one.

The tick is **per pane**, so on a multi-monitor setup each monitor's orange arm
steps at its own rate. A shared counter would have made a `cadence: 8` pass fire
at a rate that depended on the other monitors' refresh.

This is the knob that makes one bundle portable: full rate on a fast GPU, an
expensive chain at ¼ or ⅛ on vc4/v3d, same `pipeline.json`.


### `tb-chain-proof` — does the graph actually work?

Four passes that verify themselves. Each stage writes a **signature** into its own
horizontal strip and copies forward what it read, so the end of the chain carries
one strip per stage that genuinely ran, in order, having read the right target.
The final pass decodes them into three large lights and a verdict banner.

| Lights | Meaning |
|---|---|
| all green | every pass ran, in order, off the right target — the graph works |
| light *k* red | stage *k*'s signature missing or wrong: that pass did not run, or the pass after it read the wrong target and overwrote the chain |
| all red | the chain never happened — almost certainly a single-pass fallback (check the log) |

Stage 1 writes a **half-resolution** target, so a scaled intermediate carrying the
chain is covered too.

**Use this, not `bloom`, to judge correctness — including for the offload A/B.**
A difference between `tb-chain-proof` and `tb-chain-proof-inline` means the worker
path broke the chain; with bloom the same failure would look like a slightly
different glow, or like nothing at all. For *performance* A/B use `tb-chain-6` or
`tb-heavy-single`, which are heavy enough to move the numbers.

### `tb-rects-offload` — window data on the worker

The first bundle that uses window data *and* still offloads. One before-content
pass declaring only `window-rects`, so the graph is `Offload::Whole`.

Window **geometry** is plain numbers: the compositor publishes it to a shared slot
each frame and the worker reads it — no buffer import, no acquire fence, nothing
to keep alive across devices. Window **textures** are client buffers and do not
cross yet. This bundle sits exactly on that line.

It paints a halo into the background beneath each window plus a footprint
outline, so the glow spills out from behind every window edge.

| What you see | Meaning |
|---|---|
| halos track windows as you drag | working; any lag is the slot being one frame behind (documented clock divergence — measure it) |
| background but no halos | the slot is empty: no client windows, or the compositor is not publishing |
| no offload line in the log | it ran inline; check the verdict |

### `tb-windows-layer` — the `windows` built-in

`windows` is the composited **window layer**: every client window over
transparency, background excluded. It was reserved in the ABI from the start and
never implemented — a manifest naming it used to fail to load and fall back
silently. Now it is produced, consumption-gated like `history`, so a bundle that
never mentions it costs nothing.

The pass splits the screen: **left** the ordinary scene, **right** the window
layer alone over a red field.

| Right half | Meaning |
|---|---|
| windows on red, no background | correct — the layer is window-only |
| the background too | the filter leaked; not window-only |
| flat red | empty layer: no client windows, or it was not produced |

Red rather than black on purpose, so an empty layer and an opaque black window
stay distinguishable.

Beyond the effect, this is the cheap route to window content **on the worker**: by
the time windows are drawn into the layer, dmabuf and SHM surfaces have both been
resolved into ordinary images. Sharing it off-thread would move one image the
compositor owns rather than N buffers the clients own — no acquire fences, no
per-client lifetime, and SHM works for free.

### `tb-window-frost` — glass without the bindless array

The `glass` effect — frost the wallpaper behind each window, composite the window
over it — sourced from the **`windows` layer + `window-rects`** instead of
`needs: ["window-textures"]`. Compare it side by side with `glass`.

That substitution buys three things:

- **no descriptor indexing.** It runs on devices where `glass` gates out and falls
  back to the stock background entirely.
- **no index alignment.** There is no per-window array, so the whole class of
  "each window shows the other window" bugs cannot occur.
- **SHM and dmabuf identical**, because the compositor resolved both into the
  layer before the shader ever sees them.

It declares `"windows": "pipeline"`, and that is load-bearing rather than
incidental: without it `content` is the *fully composited* scene, so inside a
window the blur samples the window itself and compositing the window back over its
own blur is a no-op. The first version of this bundle did exactly that and frosted
nothing. Separated layers — background in `content`, windows in `windows` — are
what make an opaque window frostable at all, which is more than `glass` can do
(it can only frost where a window is already translucent).

The trade is real: a flattened layer can only be sampled **where the window
actually is**. An effect reading window *i* from outside window *i*'s rect — a
thumbnail, a reflection cast elsewhere — still needs the per-window array.
Frosting reads in-rect, so it doesn't.

**Tested result: it does not replace `glass`.** Frost shows the wallpaper behind a
window, including through opaque ones — which glass cannot — but it does **not**
show windows sitting *below* other windows through the frost, and glass does.

That is structural. What lies behind window *i* is the background plus windows
`0..i-1`; a flattened layer only ever offers background-plus-all-windows or
background-alone, never the prefix. Flattening destroys the ordering, so no shader
can recover it — per-window sources are the only representation that carries it.

So the two are complements, not alternatives: use the layer for a window-only mask,
for devices without descriptor indexing, or to frost an opaque window; use
`window-textures` when an effect must see between windows.

---

## Stage validation matrix

One addressable bundle per implemented stage of `SHADER_PIPELINE_WORKER.md`, so a
stage can be exercised by name. The `tb-stage-*` entries carry no shader code —
they point at the bundle that already validates that stage — except `tb-stage-5b`,
which is its own.

| Stage | Bundle | Verdict | Pass looks like | Fail looks like |
|---|---|---|---|---|
| 1 — placement verdict | *(log only)* | — | `offload=Whole/BeforeBand/None` at load, and a `warn!` per downgrade | wrong class, or a downgrade with no note |
| 2 — worker runs a chain | `tb-stage-2` | Whole | three green lights + green banner, **and** `pane running graph offthread` in the log | a red light names the stage that broke; all red = single-pass fallback |
| 3 — pipeline-owned windows | `tb-stage-3` | None | windows rotated and translucent over an intact grid | upright/opaque = flag ignored; missing = pass not drawing them |
| 5a — window geometry on worker | `tb-stage-5a` | Whole | halos track windows; the trailing distance IS the slot's one-frame lag | no halos = empty slot |
| 5b — window textures on worker | `tb-stage-5b` | Whole | bottom strip shows each window's real content, index-coloured borders | black cells = imports produced nothing; wrong window = index misalignment |
| 6 — per-pass cadence | `tb-stage-6` | Whole | cyan arm smooth, orange arm stepping in eights | both smooth = cadence ignored |
| `windows` layer | `tb-stage-w` | BeforeBand | right half shows windows on red, no background | background on the right = filter leaked |

**Stage 1 has no visual form.** A shader cannot observe where it was placed — the
output is identical either way, which is the entire point of the verdict. It is
validated by the log line and by every `-inline` twin rendering identically to its
original.

### `tb-stage-5b` is the only bundle that exercises the cross-device texture path

`glass`, `xray` and `mirror` look like they would, and do not. Each has an
after-content pass, which makes it `Offload::BeforeBand`, and `worker_can_render`
only offloads `Whole` — so all three run inline, before stage 5b and after it.
"They still work" says nothing about whether textures ever crossed a device.

`tb-stage-5b` is `Whole` **and** needs window textures, so its thumbnails can only
be drawn if a client dmabuf was imported into the worker's device, or a SHM surface
reconstructed there from an `OPAQUE_FD` share. That is the whole of 5b in one
picture.

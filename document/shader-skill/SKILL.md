# y5 background shader — skill

**Audience: an agent writing a shader bundle for the y5 compositor from a natural-language
request.** This document is the specification. It is exhaustive: everything you need to emit a
correct bundle is here. The example bundles listed at the end are for *calibration* — read one
when you want to see a shape in practice, not to discover what exists.

**This folder is self-contained.** Everything it refers to is inside it, and every path below is
relative to this file:

```
SKILL.md          this document
y5-shader-x86     a prebuilt client for the compositor's gRPC service
shader.proto      the service schema, for any other gRPC client
examples/         every bundle referenced below
```

Resolve the paths against wherever you loaded the skill from. Nothing here reaches outside the
folder, so it can be copied anywhere and still work.

Every fact here is checked against the implementation. Where something is parsed but does not
work, it says so — those are the traps that cost the most time.

---

## 0. Before you write anything

### Read first, then build

You are being asked for an effect, and this document is long. **Read the sections the request
actually needs before emitting a single line** — a bundle that has to be corrected three times is
slower and more visible to the person waiting than two minutes of reading.

Always read **§1** (which form) and **§3** (the push block). Then, by what the request involves:

| The request mentions | Read | Open |
|---|---|---|
| just a background, wallpaper, pattern | §1.1, §10 | `examples/aurora` |
| the desktop, "everything", a whole-screen look | §5, §5.1 | `examples/mp-grayscale` |
| windows — glow, outline, per-window treatment | §5, §6.1 | `examples/tb-window-frost` |
| seeing *behind* a window: transparency, refraction | §7 | `examples/tb-window-chroma` |
| focus, dragging, resizing, "the active window" | §6.2, §6.3 | `examples/tb-window-descriptors` |
| the cursor | §6.4 | `examples/tb-storage-embers` |
| several stages, blur, bloom, pyramids | §2, §11 | `examples/tb-chain-bleed` |
| trails, decay, "leaves a mark", feedback | §9.1 | `examples/tb-persist-trail` |
| particles, counters, anything accumulating | §9.2 | `examples/tb-storage-embers` |
| barrel/lens/CRT distortion, anything that bends the picture | §8 | `examples/mp-crt` |

**Open at least one example of the shape you are about to write.** The guide gives you the schema;
the example gives you the idiom — how a band loop is actually written, where the sRGB encode goes,
what a sensible `@prop` range looks like. They are short and heavily commented.

Before writing WGSL you should be able to answer, without looking anything up again: single-file
or multipass; before- or after-content; which `requires`; and whether you need to own the band.
If any of those is still open, you are not ready — go back to the table.

Then check §12 (the refusals) and §13 (the checklist) before you emit. Most first drafts fail on
something in §13.

### Reason from the primitives, not from the examples

**The examples are idioms, not a catalogue.** They show a handful of things people happened to
build. They do not describe the boundary of what is possible, and an effect being absent from them
means nothing at all.

What you can build is decided by one question: **for the pixel you are shading, what do you know?**
Under `windows: world` with geometry and textures, at every pixel you hold

- the position of that pixel, and the whole screen it sits in;
- **every** window's rectangle, crop, opacity, kind and descriptor flags — not just the one under
  the pixel;
- **every** window's texture, samplable at any coordinate you like;
- when each window opened, was focused, was resized, was moved (§6.3);
- where the cursor is and what it is holding (§6.4);
- whatever you computed last frame, if you asked for it (§9);
- and you decide, per pixel, what the final colour is.

Anything that is a function of those inputs is expressible. That is a very large space, and it
includes most things that sound physical:

| Sounds like it needs… | It does not, because |
|---|---|
| light from one window falling on another | you hold both rectangles and both textures while shading either; brightness added as a function of distance to the emitter *is* the effect |
| shadows between windows | draw order is the occluder order; darken by distance to a rect above |
| reflections of one window in another | sample the other window's texture at a reflected coordinate |
| windows reacting to each other | every window's data is available while shading every pixel |
| depth, layering, "in front of" | the band is an ORDERED list — index is your depth |
| glow, bleed, spill, refraction | resample the composited scene, or the individual windows, however you like |

The compositor is a 2D screen-space renderer. **Screen-space is the medium, not a compromise** —
there is no more "real" version of these effects available and none is being approximated away.
Do not describe such work as faking, impersonating or approximating; it is the implementation.

#### Build the whole effect; cost is not your decision

Several sections below talk about what things cost the engine. **None of that is a reason to build
a smaller effect than the one you were asked for.**

The cost material exists for two reasons, neither of which is "prefer something simpler":

- **Declaring accurately is a correctness rule.** The loader refuses a pass that reads something it
  did not declare. Declaring what you read is not thrift, it is the thing working at all.
- **The techniques in §11 make an ambitious effect affordable** — downscale before you blur, run
  the widest stage at half rate. They exist so you can build the big version, not so you settle
  for the small one.

So: build what was asked. Reach for a three-level pyramid, per-window textures, storage buffers,
whatever the effect actually needs. If the result is genuinely heavy — a full-resolution
multi-pass chain over every window, say — build it anyway and mention in one sentence that it is
doing real work and can be scaled back if their machine struggles. **That is their call, made
while looking at it, not yours made in advance.**

Silently substituting a cheaper, weaker effect is the worst outcome available: they get something
that does not match what they asked for, and no reason why.

#### The actual limits

Short, and worth knowing precisely so you never invent a limit that is not here:

- **No z-buffer and no 3D geometry.** Windows are flat rectangles in an ordered list. You can use
  the order as depth; you cannot get true per-pixel depth or perspective from it.
- **Only what is on screen, in this world.** Windows that are off-screen, minimised, on another
  world, or that a client has not drawn contribute nothing — they are not in the set.
- **Fixed capacity.** 256 entries in the geometry arrays; over that the tail is dropped.
- **Hardware.** `window_textures` needs descriptor indexing; `storage` needs
  `fragmentStoresAndAtomics`. Both are refused honestly at load, not silently.
- **No client interaction.** You are drawing; you cannot move a window, change its size or send it
  input. Read the geometry, do not expect to set it.

If a request runs into one of these, say which one and offer the nearest thing that works. If it
does not run into one of these, **it is buildable** — work out the function, do not decline.

### Talk about the effect, not the machinery

The person asking wants their desktop to look different. They did not ask about push constants,
descriptor indexing, `requires`, band ordering, or why storage buffers need a device feature.
**None of that belongs in your reply unless they ask.**

- Say in a sentence or two what you are about to make. Make it. Say whether it worked.
- Offer the knobs by their **labels** ("Cursor pull", "Trail persistence"), never by prop name or
  slot number, and only the two or three that matter.
- Mention a technical fact **only when it changes what they see or what they must do**: an effect
  their GPU cannot run, something that only shows while windows are open, a setting that resets
  when they change resolution.
- **Translate failures.** `declares storage, which this GPU cannot write from a fragment shader
  (no fragmentStoresAndAtomics)` is a message for you. To them it is "your graphics card can't do
  this one — I've used a different approach that looks close." Fix it yourself if you can; only
  surface it if they have a decision to make.
- Do not paste the manifest, the WGSL, binding tables, or your reasoning about bands and
  requirements. If they want to see it, they will ask, and the files are on disk either way.
- **Never call your own work fake.** "Impersonating", "faking", "only an approximation" — a
  screen-space compositor has no other kind of effect, so that description is both discouraging
  and untrue. Describe what it does instead: "the fire brightens whatever is near it, so windows
  next to it pick up the glow."
- **Do not say something is impossible unless it is on the limits list above.** If you are unsure,
  work out what the shader would have to read at each pixel and check it against that list. Most
  physically-worded requests are ordinary functions of data you already hold.

The exception is when they are clearly authoring rather than decorating — they name a `requires`
entry, ask why something is slow, or ask how it works. Then match them and use the real terms.

### Always tell them how to adjust it themselves

Every `@prop` you declare becomes a live slider on their desktop. **When you finish a shader, hand
over the controls** — otherwise they have to come back to you to nudge a number, which is a poor
trade for both of you.

Say it once, at the end, in plain words. Something like:

> You can tune this yourself without me — **double-click** (or **right-click**) any empty spot on
> the desktop and pick the shader icon, and the sliders appear right there while the effect is
> running. They're also in **Super+Tab → Settings → Current World** if you'd rather have the full
> panel.
>
> Try **Cursor pull** first — that's the one that changes the feel most.
>
> And any time you want a different look entirely, **Super+Tab** gets you to the shader list —
> everything built in is there alongside this one, and switching back and forth costs nothing.

Four things make that useful rather than noise:

- **Name the one or two sliders worth touching first**, by their label, and say what each changes
  in the picture. A list of nine controls is the same as no guidance.
- **The inline editor is the one to lead with.** It floats over the live desktop, so a drag shows
  its result immediately; the settings panel shows a small preview pane instead. Mention the
  settings route second, for people who prefer a real window.
- **Mention Super+Tab as the way out.** A user who does not like the effect, or who wants to
  compare it against something, should not have to ask you to change it back. Say it once, with
  the sliders.
- Say it **once**, when you hand the work over — not before you start, and not every time you
  touch the shader afterwards.

If the effect has no adjustable variables, say so plainly ("this one has no settings — tell me
what to change and I'll edit it") rather than sending them to an empty panel.

Both routes edit the same values, they save per world, and they take effect as you drag — no
restart, nothing to apply.

---

## 1. Decide the form first

There are two kinds of bundle. Choosing wrong is the most common failure.

| Ask | Form |
|---|---|
| "an animated wallpaper", "a plasma / aurora / gradient background" | **Single-file.** One WGSL file, no manifest. |
| Anything that reads the desktop, the windows, the cursor, or needs more than one pass | **Multipass.** A `pipeline.json` graph. |

A single-file bundle cannot see the desktop at all — it only draws a background. The moment the
request involves *what is on screen* ("make my windows glow", "tint everything", "CRT effect"),
you need multipass with `composited_scene`.

**Do not emit a `pipeline.json` for a plain background.** It works, but it costs an offscreen
composite the effect does not use.

### 1.1 Single-file layout

```
<bundle>/
  wgsl/shader.wgsl        # portable WGSL  (preferred)
  vulkan/shader.wgsl      # Vulkan-only WGSL
  glsl/shader.frag        # desktop GLSL 450
  gles/shader.frag        # OpenGL ES 1.00
```

Provide **one**. `wgsl/shader.wgsl` unless you have a reason. The loader tries the formats the
active renderer supports, in its preferred order, and falls back to the built-in parallax if none
compile. Vulkan is the primary renderer; GLES exists but is not the target — if you only ship
`wgsl/`, GLES sessions get the built-in background, which is acceptable.

The entry point is `fs_main`, a fragment shader. Same `Push` block as §3.

### 1.2 Multipass layout

```
<bundle>/
  pipeline.json
  passes/*.wgsl           # one file per pass (a file may be used by several passes)
  lib/*.wgsl              # optional shared modules, #import-ed
```

---

## 2. `pipeline.json` — complete schema

Every field, with its default. Unknown fields are a **hard error** (`deny_unknown_fields`) — do
not invent keys.

```jsonc
{
  "name": "my-bundle",          // required, string
  "version": 1,                 // default 1
  "category": "Multipass",      // optional; the picker heading. Omit → "User"

  "modules": ["lib/shared.wgsl"],   // default []; #import-able sources

  "targets": {                  // default {}; named offscreen images
    "blur": {
      "format": "rgba16f",      // rgba8 | rgba16f (default) | rgba32f
      "scale": 0.5,             // default 1.0; edge length vs output
      "persist": false          // default false; see §9
    }
  },

  "storage": {                  // default {}; GPU buffers, see §9
    "acc": { "bytes": 65536 }   // required; rounded up to 16
  },

  "textures": {                 // default {}; images shipped with the bundle, see §9.4
    "sheet": {
      "file": "art/sparks.png", // required; PNG, bundle-relative, no `..`
      "srgb": true              // default true; FALSE for anything read as data
    }
  },

  "windows": "engine",          // engine (default) | pipeline | world — see §7
  "decorations": "keep",        // keep (default) | off
  "letterbox": "keep",          // keep (default) | on-resize | always — CURRENTLY INERT

  "hit": { ... },               // optional pointer warp, see §8

  "passes": [                   // required, ≥1
    {
      "name": "blur",           // required
      "shader": "passes/blur.wgsl",   // required, relative to the bundle
      "inputs": { "src": "blur" },    // default {}; binding name → target name
      "output": "output",             // default "output" (= the swapchain)
      "when": "before-content",       // before-content (default) | after-content
      "defines": { "HORIZONTAL": "true" },  // default {}; naga_oil shader-defs
      "requires": ["composited_scene"],     // default []; see §5
      "place": "auto",                // auto (default) | worker | compositor
      "cadence": 1                    // default 1; run every Nth frame
    }
  ]
}
```

**`letterbox` is currently inert.** It parses and is carried, but the draw path takes the same arm
for every value. Setting it is harmless and does nothing. Do not promise a user it will change
anything.

---

## 3. The push constants

Every pass, single-file or multipass, receives this. Declare exactly this struct (you may declare
`params` shorter — `array<vec4<f32>, 1>` — if you use fewer props):

```wgsl
struct Push {
    res_zoom_time: vec4<f32>,   // xy = resolution, z = world zoom, w = time (seconds)
    pan_flow: vec4<f32>,        // xy = camera pan, zw = flow offset
    lock_alpha: vec4<f32>,      // x = lock amount, y = alpha,
                                // z = sRGB-encode flag, w = packed pan velocity
    params: array<vec4<f32>, 4>,  // 16 floats: your @prop values
};
var<immediate> pc: Push;
```

Notes that matter:

- **`res_zoom_time.xy` is THIS PASS'S target**, not the screen. A `scale: 0.25` pass sees quarter
  resolution. So `uv = frag.xy / res` is always right and needs no correction.
- **`res_zoom_time.w` is the shared animation clock**, seconds since compositor start. The same
  clock window timestamps are on (§6.2), which is what makes `t - timestamp` an age.
- **`lock_alpha.z > 0.5` means you must sRGB-encode your output.** Always end with:
  ```wgsl
  if (pc.lock_alpha.z > 0.5) { col = pow(max(col, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)); }
  ```
  Omit this and the bundle is visibly washed out on the affected path. Only the pass writing
  `output` should do it.
- **`lock_alpha.w` is the pan velocity**, two snorm16 halves:
  ```wgsl
  let vel = unpack2x16snorm(bitcast<u32>(pc.lock_alpha.w)) * 16384.0;  // world px/s
  ```

### 3.1 The world moves — `zoom` and `pan`

y5's world is **pannable and zoomable**, and `res_zoom_time.z` / `pan_flow.xy` are how a background
knows. A shader that ignores them is not neutral: it stays welded to the glass while the desktop
slides and scales underneath it, which reads as the background having come unstuck from the world.

The engine's own convention, from the stock parallax — follow it unless you mean something else:

```wgsl
// Centred, aspect-corrected by HEIGHT, then divided by zoom.
var uv = (frag.xy - 0.5 * res) / max(res.y, 1.0);
uv = uv / max(pc.res_zoom_time.z, 0.0001);
// Horizontal tracks the camera as -pan; the vertical is inverted here, which is
// the baseline the per-world "Invert pan Y" toggle flips back from.
let pan = vec2<f32>(pc.pan_flow.x, -pc.pan_flow.y);
```

- **`zoom` divides**, so zooming in makes the content bigger. Guard it (`max(zoom, 0.0001)`).
- **`pan` is in world pixels**, not UV — scale it into your own space (the parallax uses factors
  like `0.001` per depth layer, which is what makes the layers separate).
- **Depth is `pan` scaled per layer**: distant things move less. That is the whole of parallax.
- `pan_flow.zw` is a separate slow **flow** drift the engine advances on its own; it is what keeps
  a background alive while the camera is still.

#### Positions are handled for you. **Lengths are not.** This is where zoom goes wrong.

`windows.rects[i]`, `windows.srcs[i]` and `pointer.at.xy` are **screen UV, post-transform** — the
compositor has already applied pan and zoom. Never transform them again; that is a separate bug
whose symptom is windows and effects drifting apart as the user scrolls.

But that only settles *where* things are. **Every length you write yourself is a screen length by
default, and a screen length does not change when the world zooms** — while everything the engine
positioned does. So the two drift apart in *scale*, and an effect that looked right at 100 % is
visibly wrong at 50 %.

This is the mistake to watch for, because "I used the window rect the engine gave me" feels like it
should be enough, and it is not. It applies to every self-authored magnitude: a sprite's extent, a
glow radius, a noise frequency, a blur width, a border thickness, a particle size, a drift depth,
a displacement amount.

**Worked example — snow settling on windows.** Zoom out. The window halves in size, because the
engine applied the camera to its rect. The flake size, drift thickness and noise frequency were
written in screen UV, so they do not halve. The snow now looks twice as coarse relative to the
window it is sitting on. Nothing is "wrong" in the code; the lengths are simply in the wrong space.

Two correct fixes — pick by what the thing belongs to:

```wgsl
// (a) WINDOW-RELATIVE. Work inside the window's own 0..1 box. Every length is then
//     a fraction of that window and tracks it at any zoom, for free, with no `zoom`
//     read at all. Best for anything that belongs to ONE window: snow on its top
//     edge, a border, a frost pattern, wear on its surface.
let local = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));
let flake = 0.04;                      // 4% of the window, always

// (b) WORLD-SCALED. Multiply screen lengths by `zoom`, or sample fields at
//     `uv / zoom`. Best for anything that belongs to the WORLD rather than to one
//     window: sparks drifting between windows, a field, ambient particles.
let zoom  = max(pc.res_zoom_time.z, 0.0001);
let flake = 0.04 * zoom;               // a fixed WORLD size, drawn at screen scale
let field = fbm(uv / zoom);            // world-locked feature size
```

**The test, and it takes five seconds:** zoom the world out. If the effect does not shrink along
with the desktop, its lengths are in the wrong space. Do this before handing a shader over — it is
the single most common thing that looks perfect while authoring and wrong in use.

#### Screen-anchored is a legitimate choice — but make it one

A vignette, scanlines, film grain, a sheet of glass over the display: those live on the *output*,
not in the world, and should stay in screen units at every zoom. What is not acceptable is not
*deciding*. If the shader paints a place, it scales and moves with the world; if it paints the
screen, it does not. `tb-texture-paper` is the second kind, on purpose, and says so in a comment.

[`mp-parallax`](examples/mp-parallax) is the reference for the first kind — it is the stock
background, and `lib/parallax.wgsl` does zoom, pan, per-layer depth and flow in one place. Read it
rather than deriving the convention again.

### 3.2 Make it look the same at 60 Hz and 120 Hz

**Drive every animation from `res_zoom_time.w`.** It is an absolute wall clock in seconds, shared
by the compositor and the off-thread worker on purpose, so anything written as a function of `t`
looks identical at any refresh rate, on any path, on every monitor. This is the default and it is
free — `sin(t * 2.0)`, `fract(t * speed)`, `t - times.life[i].x` are all already right.

**Never advance state by a fixed amount per frame.** `x = x * 0.94` or `pos += speed` in a
`persist` target or a `storage` buffer is a per-*frame* step, and the frame rate is not a constant:
120 Hz decays twice as fast as 60 Hz, and an offloaded background runs at its own paced rate that
matches neither. The same bundle will look different on two monitors of the same desktop.

**The push carries no frame delta.** If you genuinely need one — feedback and simulation are the
only cases — store the previous timestamp yourself and subtract:

```wgsl
// One spare channel of a `persist` target (or one word of `storage`) holds the
// last frame's clock reading.
let prev = textureSampleLevel(state, samp, uv, 0.0).a;
// Clamp: the first frame reads a zeroed target, so `dt` would be the whole
// uptime; a hitch or a resume would otherwise take one enormous step.
let dt = clamp(pc.res_zoom_time.w - prev, 0.0, 0.1);
```

Then express constants **per second** and convert, rather than tuning a per-frame number:

```wgsl
let decay = pow(decay_per_second, dt);   // NOT a fixed per-frame factor
let moved = pos + velocity * dt;         // NOT pos + velocity
```

Write `pc.res_zoom_time.w` back into that channel each frame and the accumulator is rate-independent.

**`cadence` is a cost knob, never a timing mechanism.** `cadence: 2` means "every second *frame*",
so it ticks at refresh ÷ N — 30 Hz on a 60 Hz screen and 60 Hz on a 120 Hz one. It is correct for
throttling expensive work whose result is a held image; it is wrong as a way to slow an animation
down. Slow the animation by scaling `t`.

---

## 4. Bindings

```wgsl
// group 0 — the pass's own inputs. Always present if the pass has inputs or any group-1/2 block.
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var first_input: texture_2d<f32>;   // in `inputs` declaration order
@group(0) @binding(2) var second_input: texture_2d<f32>;
// …bindings 1..=N for N inputs, in the order `inputs` is written (BTreeMap → sorted by key)

// group 1 — engine blocks. Each needs its `requires` entry.
@group(1) @binding(0) var<uniform> windows: Windows;              // window_geometry / world_geometry
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>; // window_textures / world_textures
@group(1) @binding(2) var<uniform> times: Times;                  // window_times
@group(1) @binding(3) var<uniform> pointer: Pointer;              // pointer_state

// group 2 — your declared storage buffers, in SORTED NAME order
@group(2) @binding(0) var<storage, read_write> first: First;
```

**`inputs` binding order is the sorted key order**, because the manifest map is a `BTreeMap`. If
you write `{"scene": "content", "blur": "blur_a"}`, `blur` is binding 1 and `scene` is binding 2.
Name your inputs so the sorted order is the order you want, or just read the order off the sort.

**A declared texture binds here too**, in the same list and the same order — `inputs` names
targets, engine built-ins and textures from one namespace, and the shader cannot tell them apart.
There is no separate group or binding for art. See §9.4.

**A pass may bind at most 12 images**, counting inputs of every kind. That is a descriptor-set
limit, not a texture limit: Vulkan's guaranteed floor for sampled images per stage is 16. Past 12
the bundle will not load. Split the pass in two through an intermediate target.

Using `binding_array` requires `enable wgpu_binding_array;` at the top of the file.

---

## 5. `requires` — say what you read

**Nothing is inferred from use.** Sampling `content` without declaring `composited_scene` is an
error, not a quiet allocation — so this list is how a pass gets its inputs at all, and a missing
entry is a bundle that will not load.

Each entry also authorises one engine cost, which is why the engine can afford to offer them: a
bundle that declares nothing costs what a single-pass shader costs. That is an argument for
declaring *accurately*, not for declaring *little* — **declare everything the pass reads**, and do
not drop one to save work. A pass missing an input it needs is broken, not cheap.

| Entry | Binds | Cost it authorises |
|---|---|---|
| `composited_scene` | input target `content` | composites the scene offscreen first |
| `window_layer` | input target `windows` | composites a separate window layer |
| `previous_frame` | input target `history` | keeps and copies the previous frame |
| `window_geometry` | `@group(1) @binding(0)`, client windows only | collects the window set each frame |
| `world_geometry` | same binding, **every** world drawable | collects the whole world band each frame |
| `window_textures` | `@group(1) @binding(1)`, client windows only | binds window textures; **needs descriptor indexing** |
| `world_textures` | same binding, every world drawable | binds every world texture; **needs descriptor indexing** |
| `window_times` | `@group(1) @binding(2)` | tracks and uploads per-window timestamps |
| `pointer_state` | `@group(1) @binding(3)` | uploads the cursor position and buttons |

### 5.1 The three engine-provided targets

Name them in `inputs` like any target. All three are **after-content only** — a `before-content`
pass reading one is refused.

- **`content`** — the scene composited so far: background plus windows. This is what "the
  desktop" means.
- **`windows`** — the client windows over transparency, background excluded.
- **`history`** — the previous frame's composited scene. Engine-owned; you cannot choose what
  goes in it. For a feedback buffer you control, use a `persist` target (§9.1).

### 5.2 Device features

Two requirements can be refused by the hardware:

- `window_textures` / `world_textures` need **descriptor indexing**.
- Any `storage` declaration needs **`fragmentStoresAndAtomics`**.

Both are probed once at renderer init. A bundle needing an absent feature is refused at load with
a message naming it — it does not silently degrade.

---

## 6. Reading the desktop

### 6.1 Window geometry and textures

```wgsl
struct Windows {
    count: u32,
    _pad: vec3<u32>,               // vec3<u32> is 16-byte aligned: arrays start at offset 32
    rects: array<vec4<f32>, 256>,  // xy = origin, zw = size, in screen UV
    srcs:  array<vec4<f32>, 256>,  // texture-crop UV within that drawable's buffer
    attrs: array<vec4<f32>, 256>,  // x = kind, y = alpha, z = flags, w = reserved
};
@group(1) @binding(0) var<uniform> windows: Windows;
@group(1) @binding(1) var win_tex: binding_array<texture_2d<f32>>;
```

**The array length `256` is fixed and must be written exactly.** A build-time test fails any
shipped bundle that disagrees, because the arrays after a wrong one are read at the wrong offset
and every window gets another window's rectangle. Clamp with `min(windows.count, 256u)`.

Entries are **back to front** — index 0 is furthest back. To composite in draw order, iterate
`0..count` forward. To find the front-most drawable covering a pixel, iterate backward.

`attrs.x` is the kind: `0` = client window, `1` = iced-world panel. Panels are not windows —
do not chroma-key, refract or otherwise treat one as a window; blit it plainly.

Sampling a window:
```wgsl
let r = windows.rects[i];
let s = windows.srcs[i];
let local = (uv - r.xy) / max(r.zw, vec2<f32>(0.0001));
let px = textureSampleLevel(win_tex[i], samp, s.xy + local * s.zw, 0.0);
let a = clamp(px.a, 0.0, 1.0) * windows.attrs[i].y;
```

### 6.2 Descriptors — what the compositor knows about each window

`attrs.z` is a bit set. Read it as an integer:

```wgsl
let flags = u32(windows.attrs[i].z);
if ((flags & 1u) != 0u) { /* focused */ }
```

| Bit | Name | Meaning |
|---|---|---|
| `1u` | `ACTIVATED` | holds keyboard focus |
| `2u` | `TOPMOST` | the front-most window that draws |
| `4u` | `FULLSCREEN` | is fullscreen |
| `8u` | `RESIZING` | a resize is live, or the client has not committed at the new size |
| `16u` | `MOVING` | an interactive move grab is carrying it |
| `32u` | `SELECTED` | in the canvas selection |
| `64u` | `PRIMARY` | the anchor of a multi-window selection |

Every entry of one window carries the same value — a window's surface tree and its decorations are
several entries and they agree. Unknown bits are not an error.

`ACTIVATED` and `TOPMOST` are different facts: a window can be front-most and unfocused.
`MOVING` ends when the gesture ends; `RESIZING` outlives it, because the client still has to commit
at the new size.

`SELECTED` and `ACTIVATED` are different facts too, and the difference is cardinality: focus is one
window, a selection is many. It is the set a group move or resize carries and the set the select box
is drawn around, so it is the bit a bundle wants for highlighting a group rather than the focused
window. A selection of one is still a selection, and it does not have to include the focused window.

`PRIMARY` always implies `SELECTED` — it is the anchor a group operation measures from — so you may
test either bit without checking the other. **There is no primary in a selection of one:** a lone
selected window carries `SELECTED` and not `PRIMARY`, so `PRIMARY` is the right test for "is this the
anchor of a group", and the wrong one for "is this the window the user picked first".

Unlike `SELECTED` it has **no moment beside it** in §6.3. Which member is primary can change inside a
selection that is otherwise unchanged, so a timestamp there would answer "when did this become the
anchor" — a question no effect has needed, and lanes in a fixed-width uniform are not free. Animate
the anchor with the selection's own `state.z` / `state.w`.

### 6.3 Timestamps (`window_times`)

```wgsl
struct Times {
    count: u32,
    _pad: vec3<u32>,
    life:  array<vec4<f32>, 256>,  // x = opened, y = entered, z = left, w = reserved
    state: array<vec4<f32>, 256>,  // x = focused,  y = topmost,
                                   // z = selected, w = deselected
    drag:  array<vec4<f32>, 256>,  // x = resize started, y = resize ended,
                                   // z = move started,   w = move ended
};
@group(1) @binding(2) var<uniform> times: Times;
```

**These are absolute moments on the same clock as `res_zoom_time.w`, not ages.** Compute the age
yourself:

```wgsl
let age = pc.res_zoom_time.w - times.life[i].x;   // seconds since it opened
```

That is what makes an animation advance every frame on both the inline and offloaded paths. An
event that has not happened is `-1.0` (`NEVER`) — negative, so a pass that forgets to check sees a
very old event rather than one that just fired. Test with `>= 0.0`.

**`left` is only readable in hindsight.** An off-screen window contributes no entry, so nothing can
read when it left *while* it is gone. It becomes readable when the window returns — useful for
"was it away a moment or a minute", useless for an exit animation.

**`deselected` is the exception to that, and it is why the pair exists.** A window that leaves the
selection is still on screen and still contributes an entry, so unlike `left` you can read the
moment it happened while it is happening — which is what makes a selection fade-OUT possible at all:

```wgsl
let sel   = (u32(windows.attrs[i].z) & 32u) != 0u;
let since = pc.res_zoom_time.w - select(times.state[i].w, times.state[i].z, sel);
let glow  = select(1.0 - smoothstep(0.0, 0.2, since),   // fading out
                   smoothstep(0.0, 0.2, since),         // fading in
                   sel);
```

Read the flag to decide which lane you are timing from, not the other way round. Comparing the two
timestamps to infer the current state is the same answer with an extra failure mode: both are
`NEVER` until the window has been selected once, and `>=` on two `-1.0`s is not "selected".

**These give you *time since*, never *amount of*.** Anything that builds up — snow settling, wear,
a heat map, a trail — is an accumulator and needs `persist` or `storage`. A timestamp can only ever
produce a pure function of the clock, which is a different shape. **Read §9.0 before using these
for anything that grows.**

### 6.4 The pointer (`pointer_state`)

```wgsl
struct Pointer {
    at: vec4<f32>,       // xy = screen UV, z = held buttons, w = reserved
    moment: vec4<f32>,   // x = last pressed, y = last released, z / w = reserved
};
@group(1) @binding(3) var<uniform> pointer: Pointer;
```

Buttons are a bit set: `1u` left, `2u` right, `4u` middle. `!= 0u` means something is held.
Moments are absolute, like §6.3.

The position is **where the hand is**, published before any pointer warp is applied — so a warping
bundle's cursor effect stays under the cursor.

`pointer_state` implies no world set: a bundle that only wants the cursor collects nothing.

---

## 7. Ownership — `windows`

Decides who draws the world band.

- **`engine`** (default) — the engine composites windows into `content`. Your pass sees the
  finished desktop. Right for anything that only *reads*.
- **`world`** — the engine draws none of the band; your pass draws all of it, from `rects` +
  `win_tex`, in published order. Required for anything that needs to know **what is behind a
  window**: transparency, refraction, moving a window, hiding one. `content` has already
  overdrawn the background, so those are impossible without it.
- **`pipeline`** — legacy. Hands you client windows only and the engine still draws world panels
  *on top* of everything you drew. **Do not emit this.** Use `world`.

Under an ownership mode you must draw the whole band or the desktop loses windows. Also: an
ownership mode already fixes membership, so `world_geometry`/`world_textures` are refused with it
— use `window_geometry`/`window_textures`.

Ownership does not apply while the overview, the world picker or the lock screen is up; the engine
draws those itself.

### 7.1 What owning the band actually gives you

The paragraph above frames `world` around *seeing behind a window*, because that is the case that
forces it. It undersells what you get.

Owning the band means **you draw every window yourself, and while shading any pixel you hold every
window's rectangle and every window's texture.** The consequence worth stating plainly:

> **Any window can affect any other window, and any window can affect the background.**

There is no barrier between them — they are entries in one array you are looping over, and the
output colour is whatever you decide. Effects that read as physical interaction between windows are
ordinary arithmetic here:

```wgsl
// While compositing window `i`, brighten it by a field emitted from EVERY window.
// Nothing about this needs depth, normals or a second render — the emitter's
// rectangle and this pixel's position are both already in hand.
var light = 0.0;
for (var e = 0u; e < n; e = e + 1u) {
    if (e == i) { continue; }                  // or don't, for self-illumination
    let r = windows.rects[e];
    let d = rect_distance(uv, r);              // your own helper
    light = light + emit_strength * exp(-d * falloff);
}
col = col + tint * light;
```

The same shape covers spill onto the background (do it before the band loop), shadows (subtract
instead of add, gated on draw order), and reflection (sample `win_tex[e]` at a mirrored coordinate
rather than adding a scalar).

`tb-window-metaballs` in `examples/` is the reference for genuine cross-window interaction — two
windows' *fields* merge and their *contents* cross-fade in the region between them. Read it if you
want the idiom; do not read it as the limit.

---

## 8. Pointer warp — `hit`

If your shader **displaces** where things appear (barrel, lens, ripple, per-window transform), the
pointer must be corrected by the same function, or clicks land where the content is not drawn.

```jsonc
"hit": {
  "module": "lib/warp.wgsl",     // required; see the note below on "modules"
  "params": ["curve", "amount"], // @prop names, resolved BY NAME into the warp's vec4
  "evaluate": "map_static"       // map_static (default) | pointwise | map
}
```

The module defines one or both entries. **Which kind a function is falls out of its arity.**

```wgsl
// Griddable: a pure function of position. Add a 4th arg to read the clock.
fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>) -> vec2<f32>
fn hit_inverse(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>, time: f32) -> vec2<f32>

// Per-drawable. `attrs` = vec4(index, time, kind, alpha). Optional 6th arg = descriptor flags.
fn hit_drawable(uv: vec2<f32>, res: vec2<f32>, params: vec4<f32>,
                rect: vec4<f32>, attrs: vec4<f32>) -> vec3<f32>
fn hit_drawable(uv, res, params, rect, attrs, flags: f32) -> vec3<f32>
```

`hit_inverse` returns the source UV. `hit_drawable` returns `.xy` = source UV and `.z > 0.5` to
**claim** the point; the engine iterates drawables front to back and the first claim wins.

**Your render pass must `#import` the same module and call the same function.** That is the entire
point: one definition, two readers, no drift.

The warp module is read directly from the bundle, so `hit.module` does not *have* to appear in
`modules` for the warp itself to load. But `#import` resolves only against `modules`, and your
render pass has to import it — so in practice list it in both, exactly as the shipped warp bundles
do.

### First: does this bundle need `hit` at all?

**Declare it only if the displacement is large enough to make clicks visibly miss, and lasts.**
Adding `hit` is not a per-effect correction that switches itself on — the warp is a property of the
*bundle*, and once declared the engine evaluates it on **every pointer event for the whole
session**, whether the displacement is currently 5 % of the screen or exactly zero.

So a warp that only matters for a moment — a ripple for 300 ms after a click, a nudge during a
window's open animation, a lens that is only on while a key is held — is paying its full cost
continuously to correct something that is almost never there. Do not declare `hit` for those. A
click landing a few pixels off during a 300 ms transient is not a bug anyone reports; a desktop
whose cursor lags is.

`hit` earns its place when the displacement is **persistent and structural**: a CRT barrel, a lens,
a fisheye, a per-window transform — something where at rest, right now, the thing under the cursor
is not the thing being drawn there.

### Choosing `evaluate`

**Prefer `map`. Reach for `pointwise` only for a discontinuous warp.** That ordering is the
opposite of what "exact and simple" suggests, and it is the one that keeps the pointer responsive.

- **`map`** — the GPU renders a 128×128 grid as part of the frame and it is read back; the pointer
  pays a lookup. Cost is **one small pass per frame regardless of how expensive the warp is or how
  fast the pointer moves.** This is the right default for anything that animates, and the only
  sane answer for a warp with loops or many samples. It trails by one frame — 16 ms of pointer lag
  on a displacement that is itself moving, which nobody can see.
- **`map_static`** — bake the same grid once on the CPU and look it up. Best of all when it
  applies: no per-frame cost at all. Only valid for a warp whose inputs hold still, and **refused**
  for one that reads the clock (a bake of one instant is wrong every frame after it).
- **`pointwise`** — interpret the warp module on the CPU, once **per pointer event**. Pointer
  polling runs at up to 1 kHz, so this is your function, in an interpreter, on the input thread,
  hundreds of times a second. A cheap warp (the shipped barrel is ~40 IR ops) is genuinely free
  here. **An expensive one — a loop, a multi-octave field, many samples — is a stuttering cursor**,
  and it will read to the user as the whole compositor being slow rather than as a shader being
  heavy.

  Use it for exactly one reason: **the warp is discontinuous.** Both grid modes interpolate
  bilinearly between cells, which smears any jump — a kaleidoscope fold, a hard tile boundary, a
  per-region transform — across a cell and puts the pointer in the wrong region near every seam.
  A discontinuity is the one thing a 128×128 grid cannot represent at any resolution, and that is
  what `pointwise` is for. If your warp is smooth, it is not the answer, however simple it looks.

If you are unsure and the warp animates: **`map`**.

**`hit_drawable` has no choice** — it iterates the world set, so it is pointwise by necessity and
`evaluate` is ignored for it. That makes keeping it cheap a hard requirement rather than a
preference: no loops over anything but the drawables the engine already hands you, no field
evaluation, claim early. If a per-drawable warp needs to be expensive, it is the wrong shape —
express it as a griddable `hit_inverse` with `map` instead.

### 8.1 The CPU evaluator's limits

The warp module is interpreted on the CPU per pointer event. It may not sample a texture, take a
derivative, use an atomic, or read a global — pass everything as an argument. Those are rejected at
**load** with a message naming the construct. Arithmetic, branches, loops, and calls into helper
functions are all fine, including bitwise operators and integer casts.

---

## 9. State that survives the frame

### 9.0 Accumulation needs storage. Timestamps are not a substitute.

The most common wrong turn in this whole document, so it is first.

A timestamp tells you **when something happened**. From it you can compute any function of
*elapsed time* — a fade, an ease, a pulse, a ring expanding from the moment of a click. That is a
lot, and it is cheap, and for a great many effects it is the correct and complete answer.

What it cannot do is represent state that depends on **history rather than on duration**. Ask:

> Does the value I want at this pixel depend on anything other than its position, the current
> clock, and this frame's inputs?

If yes, you need `persist` (§9.1) or `storage` (§9.2). No arrangement of timestamps will get you
there, because a timestamp gives every reader the same pure function of `t` — and accumulated state
is, by definition, not a function of `t`.

Concretely, snow settling on windows:

- **Timestamps give you** "this window has existed for 12.4 s", so you can draw a snow *line* that
  thickens with age. Move the window and the snow teleports with it, fully formed. Occlude it and
  the depth keeps growing where nothing fell. Uncover it and the drift is already there. There is
  no melt, no drift, no pile against a raised edge — because there is no memory of the path, only
  of the start.
- **A `persist` target gives you** the actual effect. Depth accumulates per screen pixel where snow
  lands, decays where it does not, and stays where it settled when a window moves out from under
  it. That is snow. The whole mechanism is one target with `"persist": true`, read as last frame
  and written as this frame — see `tb-persist-trail`.

The same test catches: wear and patina, fog of war, heat maps, erosion, ink spreading, anything
that "builds up", anything that "remembers where you have been", and anything a user describes with
a verb in the perfect tense ("has been", "has collected", "has worn").

**Rule of thumb:** *time since* → timestamps. *Amount of* → `persist` or `storage`. An effect
whose description contains a quantity that grows is an accumulator, and reaching for `window_times`
instead is choosing a shape that cannot express what was asked for.

**Which of the two:** `persist` when the state lives at a *place on screen* — depth, wetness, wear,
trails. `storage` when it lives with an *entity* — per-window counters, particle lists, histograms
— because a render target can only write the pixel being shaded and a buffer can be written at any
index.

**Per-window state needs a key, and the index is not one.** `windows.rects[i]` and friends are
indexed by position in *this frame's* set, which shifts as windows open, close and reorder. The
nearest thing to a stable identity is **`times.life[i].x`, the moment the window opened**: it is
stamped once, never rewritten, and lives with the window, so it survives reordering for as long as
the window does. Hash it to pick a storage slot. Two windows first drawn on the same frame carry
the same value, so it is stable but not guaranteed unique — if a collision would be visible, mix in
something else about the window, or use a `persist` target keyed on screen position and sidestep
identity entirely.

`window_times` is still the right tool for the *events* around an accumulator — when to start
falling, when to melt faster because a resize just ended, how long since focus. Use both. Just do
not ask the timestamps to hold the depth.

### 9.1 `persist` targets — feedback

```jsonc
"targets": { "trail": { "format": "rgba16f", "persist": true } }
```

A pass may then name the same target as both an input and its output: **the read is last frame's
contents.** Implemented as a pair of images the engine alternates.

Costs nothing but memory — no device feature, works everywhere. Use for accumulation, trails,
feedback, temporal blending, grid simulation.

**Limit:** a fragment shader only ever writes the pixel it is shading. Histograms, counters and
lists are not expressible.

**Step it by `dt`, not per frame.** This is the one place the frame rate leaks into how an effect
looks: `acc = acc * 0.94` fades twice as fast at 120 Hz as at 60, and different again when the
bundle is offloaded. Keep the previous clock reading in a spare channel and use
`pow(per_second, dt)`. See §3.2.

### 9.2 `storage` buffers — scatter

```jsonc
"storage": { "field": { "bytes": 230400 }, "parts": { "bytes": 65536 } }
```

Bound at `@group(2)` in **sorted name order**, to every pass of the bundle. Read *and* write, at
any index — this is what buys scatter.

```wgsl
struct Field { cell: array<atomic<u32>, 14400>, };
@group(2) @binding(0) var<storage, read_write> field: Field;
```

Rules:

- Needs `fragmentStoresAndAtomics`; the bundle is refused where absent.
- **Budget: 64 MiB across all of a bundle's buffers.** Over it, the bundle is refused with the
  figure — not trimmed, because a shader indexes the size it declared.
- The engine zeroes each buffer **once**, at allocation. Everything after is yours: state
  surviving the frame is what an accumulator wants and what a per-frame tally must undo itself.
- **Ordering within one pass does not exist.** Invocations in a single draw have no order relative
  to each other. Clearing and accumulating in one pass produces a different arbitrary subset every
  frame — it still *looks* plausible. Split into separate passes; the engine emits a storage
  barrier after every intermediate pass.
- Use `atomicAdd`/`atomicStore` for anything more than one invocation touches. A buffer where
  invocation `i` owns index `i` needs no atomics.
- Floating point must be hand-packed as fixed point; buffer atomics are integer.
- **A simulation step is per frame unless you make it per second.** Advance positions by
  `velocity * dt`, not by `velocity`, with `dt` recovered from a stored clock reading — otherwise
  the same particles move at two speeds on two monitors. See §3.2.

### 9.3 Both are volatile

Zeroed at allocation and reallocated whenever the output size, the swapchain format or the selected
bundle changes. They are also per-device: a bundle whose placement changes restarts its
accumulation. Accumulators, not save files.

### 9.4 `textures` — images the bundle ships

The one input you cannot compute. Everything else here is generated or is the desktop; a texture is
where authored detail comes from — sprite art, a colour LUT, a scanned surface, a blue-noise table,
a logo, a gradient ramp someone drew by eye.

**It is not a new mechanism.** Declare it in `textures`, name it in a pass's `inputs` exactly as you
would name a target, and it arrives as an ordinary `texture_2d<f32>` at `@group(0) @binding(1 + n)`
in the usual sorted-by-binding-name order. No new group, no push field, nothing per frame — the
cost is one decode and one upload when the bundle is selected.

```jsonc
"textures": { "sheet": { "file": "art/sparks.png", "srgb": true } },
"passes": [{ "name": "draw", "shader": "passes/draw.wgsl",
             "inputs": { "sheet": "sheet" }, "output": "output" }]
```
```wgsl
@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var sheet: texture_2d<f32>;
```

**You must supply the file.** There is no asset library and nothing generates art for you. If the
effect needs a texture the user has not given you, either ask them for a path, generate the image
yourself and write it into the bundle (a small script beside the bundle is the honest way — see
`tb-texture-*`, which ships `make-texture-assets.py`), or build the effect procedurally instead.
Do not declare a `file` that does not exist: the bundle will refuse to load.

**`srgb` is the one attribute you have to think about.** `true` (the default) for anything drawn to
be *looked at* — the hardware linearises on sample and the art composites correctly. `false` for
anything sampled to be *used*: a LUT, a mask, a height or normal field, a noise table. Those bytes
are numbers that happen to live in an image, and linearising them corrupts every one while still
rendering something plausible. Getting it wrong is wrong output, not an error.

**Alpha is straight, as authored.** Nothing premultiplies on the way in, deliberately — a shader is
the only thing that ever sees these texels, and premultiplying would have to happen in the file's
encoded space, where it darkens every soft edge. Composite with `rgb * a`, in the shader.

Things that will bite:

- **`textureDimensions(t)`** is how you learn a texture's size. The push does not carry it.
- **The sampler is shared, `LINEAR`, `CLAMP_TO_EDGE`, no mips.** Clamping is at the edge of the
  whole image, *not* per atlas cell — sample a cell to its border and bilinear filtering pulls in
  the neighbouring frame. Inset by half a texel.
- **Repeat is `fract()` in the shader**, since the sampler clamps. That only works if the image is
  authored seamless. Sample with `textureSampleLevel(..., 0.0)`: `fract` is discontinuous at every
  tile edge, and an automatic-LOD sample reads that jump as a huge derivative and bands.
- **No mips**, so a heavily minified image aliases. Blur it into a half-scale target if that shows.

Caps — all **refused, never trimmed**, so a bundle never silently gets less than it declared:

| Cap | Value |
|---|---|
| Longest edge, per image | 4096 (checked from the file header, before decode) |
| Decoded bytes, per bundle | 64 MiB (`Σ w·h·4`, not file size) |
| Images bound by one pass | 12, counting targets and built-ins (§4) |

**PNG only.** JPEG is not supported. Paths are bundle-relative and may not contain `..` or be
absolute (unlike `modules`, which share sources across sibling bundles on purpose). A texture is
read-only — naming one as a pass `output` is an error; write to a `targets` entry instead.

A texture belongs to the **bundle**, not to a pass. Any number of passes may name the same one, in
either band, and the second use costs a descriptor and nothing else.

Editing the image file and calling `y5-shader reload` works: the file's bytes are part of the
bundle's content hash, so a repaint re-uploads.

**Read [`examples/tb-texture-all`](examples/tb-texture-all) before writing a textured bundle.** It
is the covering example — three textures, all three roles, a texture and an engine built-in mixed
in one `inputs` map so the sorted binding order is visible, one texture read by two passes in two
bands, and both traps above handled in code you can copy. The three `tb-texture-{sheet,grade,paper}`
bundles are the same material isolated one idiom at a time.

---

## 10. Variables — `@prop`

Declared in a comment in the pass that uses them:

```wgsl
// @prop name kind [default=…] [min=…] [max=…] [step=…] [label="…"] [group="…"] [choices="a,b,c"]
```

Examples:
```wgsl
// @prop amount  float default=0.5 min=0.0 max=2.0 step=0.01 label="Amount" group="Look"
// @prop style   int   default=0 choices="Wireframe,Solid,Edges only" label="Style" group="Look"
// @prop edges   bool  default=true label="Edge mode" group="Look"
```

**Slot mapping: prop #i of a pass drives `params` float slot i, in declaration order.**

```wgsl
// first prop declared
let amount = pc.params[0].x;   // slot 0
let style  = i32(pc.params[0].y + 0.5);
let edges  = pc.params[0].z > 0.5;
```

Rules and traps:

- **Use `float`, `int` and `bool` only.** The parser also accepts `vec2`, `vec3`, `vec4` and
  `color`, but **one float slot is reserved per prop** and a multi-component value delivers only
  its **first lane**. Colour props parse, appear in the settings panel, and hand the shader `.r`.
  For a colour, emit three `float` props.
- **16 slots per pass**, hard.
- `choices=` makes an `int` a dropdown. Labels may contain spaces but not commas.
- Across passes, the *union* of props (first definition wins by name) is what the user sees. Two
  passes declaring the same name share the value — which is how a threshold declared in one pass
  reaches another. Each pass's own array is indexed by *its* declaration order, not the union's.
- Per-world saved values are keyed by **name**. Renaming a prop loses the user's setting; adding or
  reordering does not.

### 10.1 `@optimized` — quality knobs

A module constant may declare a cheap value used when the world's Optimized toggle is on:

```wgsl
const FBM_OCTAVES: i32 = 5;   // @optimized 2
```

Only the literal is rewritten, so there is no second file to keep in step. The annotation must be a
bare number or the line is left alone.

---

## 11. Making an ambitious effect affordable

These are the techniques that let a big effect run, not reasons to build a small one. See §0.

### `cadence`

`"cadence": 4` runs a pass every 4th frame, holding its target in between. Refused on the `output`
pass — the frame would have no picture.

**Cadence covers a sub-chain, not a link.** Every pass writing a given target must share its
cadence, or the target alternates between the throttled result and the un-throttled one — a
per-frame flicker that reads as an unstable effect rather than a broken graph. The engine refuses
mismatched cadences on one target, naming both passes.

### `place`

`auto` (default) puts a pass on the background worker when it crosses no device boundary, else
inline. `worker` asks explicitly and is downgraded loudly when impossible. `compositor` pins it
inline. Leave it `auto` unless you have measured something.

### General

- Downscale before you blur. A `scale: 0.5` target costs a quarter of the pixels; a chain of
  halvings is how a wide blur becomes affordable.
- `rgba8` unless you need range. `rgba16f` for anything additive or HDR-ish; `rgba32f` almost never.
- Declare the requirement that MATCHES what you read: `window_geometry` if you want client
  windows, `world_geometry` if you want the whole band including panels. They differ in
  membership, not merely in price — picking the smaller one for an effect that should cover panels
  is wrong output, not a saving.

---

## 12. Rules the loader enforces

A bundle that breaks one of these is **refused at load** with a message naming the problem; the
desktop keeps its previous background. These messages are the fastest way to correct a draft.

| Refusal | Cause |
|---|---|
| requirement not declared | a pass samples `content`/`windows`/`history` without the matching `requires` |
| `world_*` with an ownership mode | `windows` is not `engine`, so membership is already decided |
| ownership without a window requirement | `windows: world`/`pipeline` but nothing to draw them from |
| cadence on `output` | the output pass must run every frame |
| mismatched cadence on one target | two passes write it at different rates |
| before-content reads `content`/`history`/`windows` | those exist only after compositing |
| writes an engine target | `content`/`windows`/`history` are engine-owned |
| storage over budget | > 64 MiB across the bundle's buffers |
| textures over budget | > 64 MiB decoded across the bundle's images, or an edge past 4096 |
| more than 12 images in one pass | inputs and textures share `@group(0)` bindings (§4) |
| texture file missing / not a PNG / path escapes the bundle | see §9.4 |
| a texture named as a pass `output`, or as both a texture and a target | textures are read-only, and the two share one namespace |
| no descriptor indexing | the device cannot bind `window_textures`/`world_textures` |
| no `fragmentStoresAndAtomics` | the device cannot write `storage` |
| warp uses a texture / derivative / global | the CPU evaluator cannot honour it |
| `map_static` on an animated warp | a bake of one instant is wrong immediately |
| unknown manifest key | `deny_unknown_fields` — check §2 |
| array length ≠ 256 in `struct Windows`/`Times` | offsets shift and every window reads another's data |

---

## 13. Checklist before emitting

1. Single-file or multipass? (§1)
2. Every `requires` matches what the pass reads — nothing missing (it will not load) and nothing
   spurious. (§5)
3. `struct Windows` / `struct Times` array lengths are exactly `256`. (§6.1)
4. `min(windows.count, 256u)` on any loop over the set.
5. `enable wgpu_binding_array;` if you use `win_tex`.
6. The pass writing `output` ends with the sRGB encode. (§3)
7. Props are `float` / `int` / `bool` only, ≤16 per pass, read in declaration order. (§10)
8. Every pass writing a target shares its cadence. (§11)
9. `windows: world`, never `pipeline`. (§7)
10. If the shader displaces the picture **persistently**, it has a `hit` block importing the same
    function — and `evaluate` is `map` unless the warp is static or discontinuous. No `hit` at all
    for a displacement that only lasts a moment. (§8)
11. Every declared texture's `file` exists in the bundle, and its `srgb` says what the bytes MEAN —
    `false` for LUTs, masks and fields. (§9.4)
12. Anything that **builds up** uses `persist` or `storage`, not timestamps. (§9.0)
13. Every animation is a function of `res_zoom_time.w`; any accumulator steps by a recovered `dt`,
    never by a fixed per-frame amount. (§3.2)
14. **Zoom test:** every length the shader authors itself — sprite size, glow radius, noise
    frequency, border width — is either window-relative, multiplied by `zoom`, or deliberately a
    screen length. Zoom out and confirm the effect shrinks with the desktop. Positions from the
    engine are already correct and must **not** be re-transformed. (§3.1)

---

## 14. Driving a running compositor — `y5-shader`

A thin CLI over the compositor's gRPC `Shader` service. **A prebuilt binary ships beside this
file — no build step:**

```sh
./y5-shader-x86 active
```

`x86` is not decoration: it is a dynamically-linked **x86-64 Linux** build against glibc. On any
other architecture, or a materially older glibc, it will not run — and the two fallbacks below
exist for exactly that.

Its source is **not** in this folder — it lives in the compositor repository at
`document/y5-shader/`, a standalone crate (own workspace, crates.io deps only) that compiles the
service's own `.proto` straight out of the tree, so the wire types cannot drift. Rebuild there
when you need a different target:

```sh
cd <repo>/document/y5-shader && cargo build --release
```

If the binary and that source ever disagree, the source wins and the binary is stale. With no
repository to hand, use the generic-client fallback below — `shader.proto` beside this file is all
it needs.

Four commands. **Each prints exactly one JSON object on stdout and nothing else.**

```sh
./y5-shader-x86 active                  # what the focused world is running
./y5-shader-x86 list                    # every bundle that can be activated
./y5-shader-x86 activate <selection>    # switch to one ("" selects the built-in)
./y5-shader-x86 reload                  # re-read the current bundle from disk
```

JSON keys are the **proto field names verbatim** — `after_passes`, not `afterPasses`.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | the call completed. **Not** "the shader works" — read `error` in the JSON. |
| `1` | could not reach the compositor, or it refused the call |
| `2` | the command line was wrong |

The split is what lets a caller tell *try again* from *fix the shader*.

### `active`

```jsonc
{
  "selection": "my-bundle",      // "" ⇒ the built-in parallax
  "path": "/home/u/.local/share/y5/background/shader/my-bundle",
  "builtin": false,              // true ⇒ compiled in, NOT editable on disk
  "error": "",                   // non-empty ⇒ the desktop is showing the fallback
  "before_passes": 1, "after_passes": 2, "targets": 3,
  "requires": ["composited_scene", "window_geometry"],
  "windows": "world",
  "props": [ { "name": "pull", "kind": "float", "label": "Cursor pull",
               "group": "Embers", "value": 0.8, "default": 0.55,
               "min": 0.0, "max": 2.0, "step": 0.01, "choices": [] } ]
}
```

`path` is where the sources are; **empty means there is nothing to edit** (the
built-in, or a `builtin:` bundle). `value` is this world's live value, `default`
is what the shader declared.

### The authoring loop

```sh
./y5-shader-x86 active | jq -r .path      # find the bundle, or "" if it is compiled in
# …write or edit the bundle…
./y5-shader-x86 activate my-bundle        # first time only
./y5-shader-x86 reload                    # after every subsequent edit
./y5-shader-x86 active | jq -r .error     # empty ⇒ it compiled
```

**`activate` and `reload` return before the bundle is compiled** — the load happens
on the compositor's next frame. So the compile result is always read back with a
second `active` call, never inferred from the first response. `error` carries the
same message §12 describes, which is what makes the refusals a usable feedback
channel rather than a log line.

`reload` re-reads the *same* selection from disk. Nothing else invalidates a loaded
bundle: the compositor has no reason to stat one it has already compiled.

To write a new bundle, put it in the shader directory —
`${XDG_DATA_HOME:-~/.local/share}/y5/background/shader/<name>/` — and `activate`
it by folder name. A `builtin:` bundle cannot be edited; copy it to a new folder
first.

The socket is `/tmp/y5-compositor-rpc.sock`, overridable with `Y5_RPC_SOCKET`.

### If `y5-shader-x86` will not run

Two fallbacks, in order: **rebuild it** from the repository if you have one and a cargo
toolchain, or use **any gRPC client** — which needs nothing beyond this folder.

The CLI is a convenience, not a dependency. **Anything that speaks gRPC can drive the same service** —
the CLI has no privileged access, it just formats the output. Fall back to a generic client when the prebuilt binary
is the wrong architecture, or there is no repository or toolchain to rebuild it with.

What you need to connect:

| | |
|---|---|
| transport | Unix domain socket, HTTP/2, **plaintext** (no TLS) |
| address | `/tmp/y5-compositor-rpc.sock` |
| service | `y5.compositor.rpc.protocol.client.shader.Shader` |
| methods | `active`, `list`, `activate`, `reload` |
| proto | `shader.proto` (beside this file) |
| import path | `.` (this folder) |

**There is no server reflection.** The compositor registers the four services and nothing else, so
a client cannot discover the schema over the wire — you must point it at the `.proto` above. A
generic client that fails with "server does not support the reflection API" is telling you this,
not that the socket is wrong.

With `grpcurl`:

```sh
SVC=y5.compositor.rpc.protocol.client.shader.Shader

grpcurl -unix -plaintext -proto shader.proto \
        /tmp/y5-compositor-rpc.sock $SVC/active

grpcurl -unix -plaintext -proto shader.proto \
        -d '{"selection":"my-bundle"}' \
        /tmp/y5-compositor-rpc.sock $SVC/activate

grpcurl -unix -plaintext -proto shader.proto \
        /tmp/y5-compositor-rpc.sock $SVC/reload
```

With Python:

```sh
pip install grpcio grpcio-tools
python -m grpc_tools.protoc -I . --python_out=. --grpc_python_out=. shader.proto
```

```python
import grpc
import shader_pb2 as pb, shader_pb2_grpc as rpc

ch = grpc.insecure_channel("unix:///tmp/y5-compositor-rpc.sock")
stub = rpc.ShaderStub(ch)
print(stub.active(pb.ActiveRequest()))
stub.activate(pb.ActivateRequest(selection="my-bundle"))
stub.reload(pb.ReloadRequest())
```

#### Two differences from `y5-shader`'s output — both will bite

1. **Field names.** `y5-shader` prints the proto field names verbatim
   (`after_passes`, `before_passes`). Protobuf's *canonical* JSON mapping is lowerCamelCase, and
   most generic clients follow it — so expect `afterPasses` from `grpcurl`. Read the keys you
   actually get rather than assuming either spelling.

2. **Empty fields may be absent entirely.** proto3 JSON omits fields at their default value unless
   the client is told otherwise (`grpcurl -emit-defaults`). So `"error": ""` will usually not
   appear at all — and the check that matters most in this whole document, *did the shader
   compile*, is exactly a test for an empty `error`. **Treat a missing `error` key as success**, or
   pass the client's emit-defaults flag so the field is always present.

   `y5-shader` does not have this problem: it serialises the whole message, so `error` is always a
   key.

Everything else — the semantics, the exit-code distinction between "could not call" and "the
shader did not compile", the fact that `activate`/`reload` return before the bundle is compiled —
is a property of the service, not of the client, and holds whichever you use.

---

## 15. Example bundles — idioms, NOT a catalogue

**These are idioms, not a menu.** They exist so you can see how a shape is written — where the
sRGB encode goes, how a band loop reads, what a sensible `@prop` range looks like. They are a
handful of things somebody built, not a description of what the pipeline can do, and **an effect
being absent from this table says nothing about whether it is possible.** Decide what is possible
from §0 and the data model; use these to get the idiom right.

All of them are in `examples/`, including the `mp-*` set, which in the compositor ships compiled
INTO the binary — those are copies of the shipped sources, selectable at runtime as
`builtin:mp-<name>` and not editable in place.

| Bundle | Utilises |
|---|---|
| [`aurora`](examples/aurora) | single-file WGSL, no manifest, `@prop` — the "don't over-engineer" case |
| [`mp-parallax`](examples/mp-parallax) | multipass with **no** requirements; the shared `lib/parallax.wgsl` the rest `#import`. Also **the `zoom`/`pan` reference** — per-layer depth, flow drift, the engine's coordinate convention (§3.1) |
| [`mp-grayscale`](examples/mp-grayscale) · [`mp-invert`](examples/mp-invert) · [`mp-colorblind`](examples/mp-colorblind) | the minimal after-content shape: `composited_scene`, one filter pass, `category` |
| [`mp-levels`](examples/mp-levels) | `world_geometry` **without** owning — per-window treatment while the engine still draws |
| [`mp-effects`](examples/mp-effects) | the widest surface: `windows: world`, 3 passes, intermediate target, `window_geometry` + `window_textures` + `composited_scene` |
| [`mp-crt`](examples/mp-crt) | after-content warp + `hit` `map_static` + the cursor correction; resolution/palette quantisation |
| [`tb-chain-bleed`](examples/tb-chain-bleed) | 10 passes / 6 targets / 4 files: `defines`, ping-pong, `scale` pyramid, `cadence`, `int`+`choices`, `bool` |
| [`glass`](examples/glass) | `world_geometry` + `world_textures` + `previous_frame` together; bindless sampling |
| [`motion-blur`](examples/motion-blur) | `previous_frame`, minimal; the velocity lane |
| [`tb-window-chroma`](examples/tb-window-chroma) | `windows: world` — the case that is impossible without ownership |
| [`tb-window-metaballs`](examples/tb-window-metaballs) | **windows affecting each other**: fields merge between windows and their contents cross-fade in the bridge |
| [`tb-window-descriptors`](examples/tb-window-descriptors) | every descriptor flag, every timestamp, and `pointer_state`, mapped to distinct visuals |
| [`crt-rotating-input-both`](examples/crt-rotating-input-both) | `hit_drawable` — per-drawable warp; the only one |
| [`tb-crt-spin`](examples/tb-crt-spin) · [`crt-input-map-live`](examples/crt-input-map-live) | `evaluate` `pointwise` and `map`, against `map_static` |
| [`tb-persist-trail`](examples/tb-persist-trail) | `persist` — a target read as last frame's |
| [`tb-storage-embers`](examples/tb-storage-embers) | `storage` ×2, atomics, scatter, barriers, `pointer_state`; two buffers wanting opposite persistence |
| [`tb-storage-histogram`](examples/tb-storage-histogram) | the minimal storage probe — reset / tally / read as three passes |
| [`tb-texture-all`](examples/tb-texture-all) | **the covering texture example** — three textures, all three colour-space roles, a texture and a built-in mixed in one `inputs` map, one texture read by two passes in two bands, a bundle-wide `Show` prop declared in both. Copy from this one. |
| [`tb-texture-sheet`](examples/tb-texture-sheet) | `textures` as **art**, isolated — a sprite atlas animated off the clock; half-texel cell inset, straight alpha |
| [`tb-texture-grade`](examples/tb-texture-grade) | `textures` as **data**, isolated — a colour LUT strip, `"srgb": false`, after-content |
| [`tb-texture-paper`](examples/tb-texture-paper) | `textures` as a **material**, isolated — seamless tiling via `fract()`, height + slopes, lit by `pointer_state` |
| [`tb-window-frost`](examples/tb-window-frost) | `window_layer` |
| [`tb-window-xray`](examples/tb-window-xray) | `int` props with `choices` |
| [`tb-cadence`](examples/tb-cadence) | `cadence` in isolation |
| [`broken-wgsl`](examples/broken-wgsl) | what a compile error does — falls back, does not crash |

**Not covered on purpose:** `windows: "pipeline"` (legacy; §7) and the GLES/`mtx-*` bundles
(GLES is a fallback path, not a target). Their absence is a scope decision, not a gap.

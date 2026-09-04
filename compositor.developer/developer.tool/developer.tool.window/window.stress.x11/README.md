# window.stress.x11 — X11 / XWayland stress harness

A two-process developer tool for exercising y5's **native XWayland** path: the
in-process X11 window manager, override-redirect handling, focus, placement, the
selection bridge and per-window process identity.

Sibling of `../window.stress`, which does the same job for wayland clients, and built
the same way: standalone crate, own `Cargo.lock`, crates.io deps only, **not** in
`link.all.sh`.

```
x11-stress-controller   spawns and drives the subject (terminal, not a GUI panel)
x11-stress-subject      the X11 client under test
```

The subject speaks **only X11** — it never touches wayland. Everything it does reaches
y5 through Xwayland and the in-process XWM, which is the path under test. It is built
on `x11rb`, the same binding smithay's own XWM uses, so the requests it sends are
exactly the shape the compositor is written against.

The controller is a terminal program rather than the button panel the wayland harness
uses: here the things being looked at are X11 windows, and a wayland control window
would just be one more surface in the way. It also works over ssh and pipes into a
script.

## Build & run

```bash
cargo build --profile release-fast
T="$(cargo metadata --no-deps --format-version 1 | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')"
DISPLAY=:1 "$T/release-fast/x11-stress-controller"        # :N from y5's `XWayland ready on :N`
```

> The repo's `.cargo/config.toml` pins ONE `target-dir` for the whole checkout and it
> applies here too — the binaries land in `compositor.kernel/kernel.loader/target/`,
> not in a `target/` beside this crate. The `cargo metadata` line resolves it wherever
> it is.

Headless check of the command vocabulary — no X server, no compositor, no subject:

```bash
x11-stress-controller --selftest
```

Scripting / CI repros — drive the subject directly:

```bash
( printf 'map 400 300\n'; sleep .5; printf 'or 1 60 40 200 160\n'; sleep .5;
  printf 'info\n'; sleep .3; printf 'quit\n' ) | x11-stress-subject
```

## Scenarios

`--list` prints these; `--scenario <name>` replays one and then drops into interactive
mode with the windows still up.

| name | what it is for |
|---|---|
| `basics` | one managed window: title, class, resize, fullscreen |
| `identity` | does the compositor see the **app's** pid, not the X server's |
| `focus` | all four ICCCM input models — the silent no-keyboard case |
| `icon` | `_NET_WM_ICON`: which image the decoder picks, and what survives a bad one |
| `translucent` | a 32-bit ARGB window: is what is **behind** it still drawn |
| `popup` | a **real** y5 popup: override-redirect + `WM_TRANSIENT_FOR` + menu type |
| `popup-geometry` | a popup that moves and resizes **itself**, the way a real menu does |
| `popup-input` | does the popup get crossings and clicks where it overlaps its parent |
| `types` | every `_NET_WM_WINDOW_TYPE` in turn: which get popup treatment |
| `type-flip` | the classification changes between map and surface association |
| `self-geometry` | toplevel vs transient vs override-redirect moving/resizing themselves |
| `or-menu` | an override-redirect menu on a window: does it draw and take clicks |
| `or-fight` | a client that re-anchors its own menu — the provisional decision's risk |
| `or-submenu` | override-redirect chained off an override-redirect |
| `placement` | the compositor must honour a size request and **refuse** a move |
| `transient` | `WM_TRANSIENT_FOR` — the window should size itself, not be locked |
| `close` | `WM_DELETE_WINDOW` vs a window that does not offer it |
| `stacking` | two windows plus a menu: does the X server agree about the order |
| `clipboard` | X11 takes the clipboard, then reads it back |
| `storm` | rapid resize + re-anchor: is the configure flush coalescing |

## What each one is checking

- **`identity`** is the reason native XWayland was worth doing. Under
  `xwayland-satellite` every X11 window's `wl_surface` belonged to the one proxy
  client, so every window reported the proxy's pid and Steam attribution, `Y5_TEARING`
  and the tearing tag never fired. The subject prints its real pid on startup and sets
  `_NET_WM_PID` on every window; the compositor's own window dump must agree.
- **`focus`** catches the failure that is invisible until you try to type. A compositor
  whose seat focus is a plain `wl_surface` never reaches smithay's
  `KeyboardTarget for X11Surface`, so unless it calls `set_input_focus` itself the X
  server is never told who has focus and X clients get **no keyboard input at all**.
  The four models take different paths through that call, so all four get a window.
  The subject logs `KEY keycode=…` when a keypress actually arrives.
- **`icon`** is the only thing that exercises the `_NET_WM_ICON` decoder, which has more
  branches than it looks: a property carries several sizes in any order, and the rule is
  *the smallest image at least 64 across, else the largest*. Each image carries **two**
  colours, because the icon has to answer two questions at once: a thick border in the
  owning window's own background colour, so an icon in a dock, an overview or a
  placeholder can be matched back to the window it came from, and a centre colour-coded
  **by size** — 16 red, 32 orange, 48 yellow, 64 green, 128 blue, oversized white — which
  is what says which image out of the property the compositor picked. Nothing is
  instrumented on either side; you read both off the icon. Seven windows, one per shape, each set **before** its map
  (what real applications do): `single`→green, `pyramid`→green (not the first listed and
  not the largest), `large-first`→white, `oversize`→yellow (the 2048 image must be skipped
  *and the walk continued*), `truncated`→green (a bad header ends the walk but keeps what
  it already has), `zero`→**no icon** (zero dimensions end the walk, so the good image
  after them is unreachable — this one is here to prove that, not to pass), `alpha`→a
  clean fade (straight alpha; premultiplied misread as straight darkens the transparent
  edge). The eighth window is the LATE case and the likeliest to disagree: the icon is set
  *after* the map, and the compositor caches a window's icon in a `OnceLock` — if it has
  already answered "no icon", a later property never appears.
- **`translucent`** is the only window here with a **non-opaque region**, and the depth is
  the whole point. An X11 window on the default 24-bit visual has no alpha channel, so
  Xwayland can mark its `wl_surface` fully opaque; a depth-32 window carries per-pixel
  alpha and cannot be. That opaque region is what a compositor's occlusion culling reads,
  so this arrangement — a translucent window over an opaque one — says whether y5 still
  draws what is *behind* it. The window is painted as vertical alpha bands, clear on the
  left to solid on the right, inside an opaque frame, and the three outcomes look
  different on purpose: **correct** is the window beneath showing through on the left and
  fading out to the right; **culled** is the frame present with black or garbage instead
  of the window beneath; **alpha misread** is a ramp that washes out toward the
  transparent end rather than fading, which is premultiplied colour treated as straight.
  The frame matters — a fully transparent edge is otherwise indistinguishable from
  "nothing drawn" and from "culled". The last two steps are the *other* transparency:
  `_NET_WM_WINDOW_OPACITY` is a compositing-manager convention applied to a window with no
  alpha channel at all, and ignoring it is a defensible answer — this only says which one
  y5 gives.
- **`popup` / `popup-geometry` / `popup-input`** are the popup path proper, and the
  distinction that is easy to miss: an X11 window becomes a y5 **popup** only when
  THREE things hold together — it is ephemeral (override-redirect, `_NET_WM_STATE_MODAL`
  or a menu/tooltip/dnd type), it names a resolvable `WM_TRANSIENT_FOR`, and it is not
  output-sized. Drop any one and it is an ordinary window instead. A popup gets no
  uuid, no Space slot, no decoration and no placeholder; it lives in the `PopupManager`
  and is drawn and hit through `popups_for_surface`.
  - `popup-geometry` is the part a menu actually does at runtime: re-anchor when it
    would run off an edge, resize when its content changes. Both arrive as a plain
    `ConfigureWindow` that no window manager is consulted about. The last step sends
    position and size in ONE request, which is where a compositor handling the two on
    separate paths comes apart.
  - `popup-input` answers "who gets the click" in the overlap. The `ENTER` / `LEAVE`
    lines are the sensitive signal — they name the window the X server's own hit test
    picked, without needing a click to have gone missing first — and `pointer` asks the
    server the same question directly. Where they disagree with what is on screen,
    `stack` has the reason: the X stack is the only lever y5 has over which client an
    ungrabbed pointer event reaches.
- **`types`** puts one child of every `_NET_WM_WINDOW_TYPE` on screen at once, so the
  classification can be read rather than guessed. Menu, popup-menu, dropdown-menu,
  tooltip, dnd and combo should come out as popups; dialog, utility, toolbar, normal
  and dock should stay windows. A *fullscreen-sized* child is never a popup regardless.
- **`type-flip`** is the one that pokes at an assumption rather than a feature. The
  classification is taken **twice** — at the map request, and again when the wl_surface
  associates — and every input to it except override-redirect is an ordinary X property
  the client may change in between. This scenario changes it in both directions. A
  window that ends up queued twice, or never queued at all, shows up here.
- **`self-geometry`** puts the three shapes side by side under the same request.
  Expected: all three keep the **size** they ask for; only the override-redirect one
  actually **moves**. For a managed window the same call is redirected to the WM as a
  `ConfigureRequest`, which y5 refuses for position — placement is the compositor's.
- **`or-menu` / `or-fight` / `or-submenu`** are the provisional decision, and are
  deliberately the *parentless* override-redirect path — no `WM_TRANSIENT_FOR`, so
  never a popup. y5 currently
  treats override-redirect surfaces as ordinary windows — same uuid, placeholder,
  decoration, resize and scale — rather than filtering them as chrome. `or-fight` is
  the case that decides whether that holds: the client keeps moving its own menu while
  the compositor is placing it. Watch whether it settles, jitters, or walks off.
- **`placement`** pins the contract: a `ConfigureRequest` **size** is honoured (it is
  how an X11 window says how big it wants to be), a **position** is refused (placement
  is the compositor's, exactly as for an xdg toplevel).
- **`close`** exercises both halves of `shell::close`. A window that offers
  `WM_DELETE_WINDOW` is asked politely; one that does not is destroyed outright, which
  is what every X window manager does — and the compositor must **not** escalate to
  signalling the pid on top of that. Under a proxy that pid was the X server; here it
  is this whole subject process, so the giveaway is the other windows vanishing too.
- **`storm`** checks the per-frame configure flush is coalescing rather than answering
  every request. Watch the `configured by the compositor` lines: a few per second is
  right, hundreds is the flush not doing its job.

## Reading the output

Both processes tag stderr. The subject reports what the **server** thinks, not what it
asked for, because the divergence is the point:

```
[subject] 1 configured by the compositor: 640x400+120+80 (was 300x200+40+40)
[subject]   1 x11=0x01000003 or=false asked=300x200 server=640x400 at +120+80 label="win 1"
```

`info` uses `translate_coordinates` against the root rather than `get_geometry`,
because under a reparenting window manager the latter is relative to a frame and not
to the root.

### Known baseline: the satellite

Run against the old `xwayland-satellite` deployment, every window reports `at +0+0` —
the proxy pins all toplevels to their monitor origin (its own `ARCHITECTURE.md` says
so), including override-redirect ones, which is why X11 menus landed in the wrong
place. Native XWayland should show real, distinct coordinates. That contrast is the
quickest smoke test that you are talking to the right X server.

## Command vocabulary

`help` at the interactive prompt prints this. Window ids are the harness's own small
integers (1, 2, 3 … in creation order), not X11 window ids — a script has to be able to
name a window before the server has assigned one. `info` lists the mapping.

| group | commands |
|---|---|
| managed | `map <w> <h>`, `map-argb <w> <h>`, `map-transient <id> <w> <h>`, `destroy <id>` |
| transparency | `map-argb` (32-bit visual, per-pixel alpha), `opacity <id> <0-255>` (`_NET_WM_WINDOW_OPACITY`) |
| identity | `title <id> <text>`, `class <id> <name>`, `pid <id> <n>` (`0` clears) |
| state | `fullscreen <id> on\|off`, `input-model <id> none\|passive\|locally\|globally`, `close-mode <id> delete\|nodelete` |
| children | `map-child <parent> <dx> <dy> <w> <h> or\|managed <type>`, `or <parent> <dx> <dy> <w> <h>`, `or-reanchor <id> <dx> <dy>`, `or-chain <id> <dx> <dy>` |
| icon | `icon <id> <spec>` (after map), `icon-next <spec>` (before the next map) |
| classification | `window-type <id> <type>`, `modal <id> on\|off`, `skip-taskbar <id> on\|off`, `transient-for <id> <parent\|0>` |
| geometry (client asks) | `resize-request <id> <w> <h>`, `move-request <id> <x> <y>`, `raise <id>`, `lower <id>` |
| geometry (client does) | `self-move <id> <x> <y>`, `self-resize <id> <w> <h>`, `self-move-resize <id> <x> <y> <w> <h>` |
| selection | `copy <text>`, `paste` |
| storms | `storm-resize <id> <n>`, `storm-reanchor <id> <n>` |
| introspection | `info`, `stack`, `pointer`, `quit` |

`<type>` is one of `unset`, `normal`, `dialog`, `menu`, `popup-menu`, `dropdown-menu`,
`tooltip`, `notification`, `splash`, `dnd`, `combo`, `utility`, `toolbar`, `dock`.
`unset` deletes `_NET_WM_WINDOW_TYPE`, which is not a gap but the interesting case: with
no declared type the classifier falls back to `skip-taskbar && !WM_DELETE_WINDOW`.

`<spec>` for the icon commands is one of `none`, `single`, `pyramid`, `large-first`,
`oversize`, `truncated`, `zero`, `alpha` — see the `icon` scenario for what each expects.

`map-child` sets the type **before** the map request, which is when the compositor first
classifies. `window-type` sets it after, which is a different test — see `type-flip`.

The two geometry rows are the same X request with different meanings. For an
override-redirect window a `ConfigureWindow` is not a request at all: no window manager
is consulted and the server just does it. For a managed window it is redirected to the
compositor as a `ConfigureRequest`. The `self-*` commands say which is expected in the
log line they print, so a wrong answer reads as a contradiction rather than a number.

Managed windows are drawn in saturated colours, override-redirect ones in pale ones,
each labelled with its id, title and current size, so a screenshot is readable without
counting. Labels use the core `fixed` font — no font dependency; if the server has no
such font the harness still runs, just without labels.

# tb-crt-offload

`tb-crt`, with one word changed: `"place": "worker"` on the `crt` pass. Same two
shader files — this bundle points at `../tb-crt/passes/`, so the pair can never
drift apart.

## What it is for

`tb-crt` and this are the A/B for the **present path**. They render the identical
picture by opposite routes:

| | `tb-crt` | `tb-crt-offload` |
|---|---|---|
| verdict | `BeforeBand` | `AfterBand` |
| backdrop (colour bars) | **worker** | **inline** |
| CRT pass | **inline** | **worker** |
| what fills the swapchain | the local after pass | the worker's decorated band |
| lag | none on the CRT, ~1 frame on the backdrop | ~2 frames on the whole picture |

If the two look the same apart from latency, the present path is correct. If
`tb-crt-offload` shows a frozen picture, the band is stale and the withdraw path
failed. If it shows the colour bars with no tube artefacts, the worker's tail
never ran and the compositor fell through to the passthrough.

## Why the backdrop flips to inline

Nobody asked it to. `backdrop` says nothing about placement, so it is `auto`, and
on its own `auto` would put it on the worker — that is exactly what happens in
`tb-crt`. Here the tail is claimed, and **a graph cannot have both ends on the
worker**, so the head is given up.

That is not a heuristic, it is the shape of the dependency. The tail samples
`content`, and `content` is what the COMPOSITOR produces by compositing windows
over the background. So:

```
head (worker) -> background band -> compositor composites windows -> content
content -> (worker) tail decorates -> band -> compositor presents
```

For the worker to run both in one pass it would have to render the head, then
**block mid-pass** until the compositor had composited windows on top of it, then
render the tail. That is a synchronous cross-queue handshake — precisely the
design the whole feature is built to avoid (`worker.render`, `publish.ring`,
`worker.serve` all state the same law: completion-before-publish, never a
cross-queue semaphore handshake). And even if it were free, the picture would
have crossed the device boundary three times before reaching the screen, so the
background you were looking at would be several frames older than the effect
applied to it.

Giving up the head costs one background pass on the compositor. Keeping it would
cost a handshake. So: **for an `AfterBand` graph, every before-content pass runs
inline** — `place()` moves them, and says so in `notes` only when a pass had
explicitly asked for the worker (an `auto` pass moving is the resolution working,
not a downgrade).

## What "backdrop" is here

`tb-crt`'s own first pass: SMPTE-ish vertical colour bars with a darkened lower
band. It is the *background* — saturated colour and hard edges, chosen so the CRT
pass has something whose distortion is obvious. Client windows are composited over
it by the engine, and the CRT pass then treats background and windows alike, which
is the whole point of decorating `content` rather than a background.

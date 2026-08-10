#!/usr/bin/env python3
"""Generate the PNG art the `dragons` bundle ships.

Two atlases, both straight alpha, both sRGB colour:

  art/dragon.png   4x4 = 16 frames of one wingbeat cycle, dragon facing RIGHT
  art/flame.png    4x4 = 16 frames of a fire puff igniting, rolling and dying

Kept as a script beside the bundle so the art is reproducible and reviewable as
code rather than as an opaque binary. Stdlib only — no numpy, no PIL.

    python3 make-dragon-assets.py            # ~2-4 minutes, pure Python

DRAWING APPROACH
----------------
Everything is a signed distance field: tapered capsules for the spine, neck,
tail and limbs, an ellipse for the skull, a scalloped polygon for each wing
membrane. Coverage comes from the distance itself (a smoothstep across one
texel), so edges are antialiased analytically rather than by supersampling —
which is what keeps a pure-Python rasteriser to a few minutes at 256px cells.

The realism is in the shading, not the outline. Four things do most of the work:

  * FORM. A thickness proxy (how far inside the silhouette a pixel sits) drives
    a cylindrical falloff, so limbs read as tubes rather than flat cut-outs.
  * LIGHT TEMPERATURE. Cool skylight from above, warm bounce from below, and a
    hot rim along the upper silhouette — the separation is what stops a dark
    animal reading as a sticker.
  * OCCLUSION. The membrane darkens toward the body and where the wing roots
    meet the shoulder; contact shadow is most of what glues parts together.
  * TEXTURE. fbm mottling over the hide, ventral scutes across the belly, and
    veins fanning from the wrist through the membrane. No hard outlines
    anywhere, and no flat fills.
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ART = os.path.join(HERE, "art")

# The dragon atlas is TWO-DIMENSIONAL: a column per wingbeat frame, a row per
# roll angle. The shader picks a cell with (flap, roll) rather than one index.
FLAP_STEPS = 8             # one full wingbeat
ROLL_STEPS = 12            # a full 360 degree roll, 30 degrees apart
MOUTH_STEPS = 2            # jaws shut / jaws open — an animal breathing fire
DRAGON_CELL = 192          # (8*2) x 12 x 192 = 3072x2304, 28 MiB decoded

COLS, ROWS = 4, 4          # the flame puff atlas
FLAME_CELL = 128           # 4x4 x 128 = 512x512

# The breath atlas is a JET, not a puff: wide cells, mouth at the left edge.
BREATH_COLS, BREATH_ROWS = 4, 4
BREATH_W, BREATH_H = 256, 128   # 4x4 -> 1024x512


# ----------------------------------------------------------------- png / colour
def write_png(path, w, h, rgba):
    """rgba: flat bytearray, w*h*4, straight alpha."""
    raw = bytearray()
    for y in range(h):
        raw.append(0)  # filter type 0
        raw += rgba[y * w * 4:(y + 1) * w * 4]

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    print(f"  {os.path.relpath(path, HERE)}  {w}x{h}")


def srgb(x):
    """linear 0..1 -> sRGB byte. Both sheets are art, so they ship sRGB-encoded."""
    x = 0.0 if x < 0.0 else (1.0 if x > 1.0 else x)
    v = 12.92 * x if x <= 0.0031308 else 1.055 * (x ** (1 / 2.4)) - 0.055
    return int(round(v * 255))


def clamp(x, a=0.0, b=1.0):
    return a if x < a else (b if x > b else x)


def mix(a, b, t):
    return a + (b - a) * t


def mix3(a, b, t):
    return (mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t))


def smoothstep(e0, e1, x):
    t = clamp((x - e0) / (e1 - e0 + 1e-9))
    return t * t * (3.0 - 2.0 * t)


def hash01(n):
    return (math.sin(n * 127.1) * 43758.5453) % 1.0


# ------------------------------------------------------------------- primitives
def sd_seg(px, py, ax, ay, bx, by):
    """Distance to segment AB, and the projection parameter along it."""
    vx, vy = bx - ax, by - ay
    wx, wy = px - ax, py - ay
    vv = vx * vx + vy * vy
    t = 0.0 if vv <= 1e-12 else clamp((wx * vx + wy * vy) / vv)
    dx, dy = wx - vx * t, wy - vy * t
    return math.hypot(dx, dy), t, (dx, dy)


def sd_capsule(px, py, a, b, ra, rb):
    """Tapered capsule. Returns (signed distance, local radius, t, offset)."""
    d, t, off = sd_seg(px, py, a[0], a[1], b[0], b[1])
    r = mix(ra, rb, t)
    return d - r, r, t, off


def sd_ellipse(px, py, c, rx, ry, rot_=0.0):
    dx, dy = px - c[0], py - c[1]
    if rot_ != 0.0:
        ca, sa = math.cos(-rot_), math.sin(-rot_)
        dx, dy = dx * ca - dy * sa, dx * sa + dy * ca
    k = math.hypot(dx / rx, dy / ry)
    r = min(rx, ry)
    return (k - 1.0) * r, r, 0.5, (dx, dy)


def sd_poly(px, py, pts):
    """Signed distance to a closed polygon (negative inside)."""
    d = 1e9
    inside = False
    n = len(pts)
    j = n - 1
    for i in range(n):
        ax, ay = pts[i]
        bx, by = pts[j]
        dd, _, _ = sd_seg(px, py, ax, ay, bx, by)
        if dd < d:
            d = dd
        if (ay > py) != (by > py):
            xint = (bx - ax) * (py - ay) / (by - ay + 1e-12) + ax
            if px < xint:
                inside = not inside
        j = i
    return -d if inside else d


def rot(v, ang):
    ca, sa = math.cos(ang), math.sin(ang)
    return (v[0] * ca - v[1] * sa, v[0] * sa + v[1] * ca)


def norm(v):
    m = math.hypot(v[0], v[1]) + 1e-12
    return (v[0] / m, v[1] / m)


def add(a, b, s=1.0):
    return (a[0] + b[0] * s, a[1] + b[1] * s)


# ----------------------------------------------------------------------- noise
def vnoise(x, y, seed):
    xi, yi = math.floor(x), math.floor(y)
    xf, yf = x - xi, y - yi
    u = xf * xf * (3 - 2 * xf)
    v = yf * yf * (3 - 2 * yf)

    def at(ix, iy):
        return hash01(ix * 37.0 + iy * 91.7 + seed * 13.3)

    a, b = at(xi, yi), at(xi + 1, yi)
    c, d = at(xi, yi + 1), at(xi + 1, yi + 1)
    return mix(mix(a, b, u), mix(c, d, u), v)


def fbm(x, y, seed, octaves=3):
    amp, freq, total, tot_a = 0.5, 1.0, 0.0, 0.0
    for _ in range(octaves):
        total += vnoise(x * freq, y * freq, seed) * amp
        tot_a += amp
        amp *= 0.5
        freq *= 2.03
    return total / tot_a


# ------------------------------------------------------------------ the palette
# Linear values. A slate-olive animal with a warm ochre underside: dark enough to
# read as a silhouette against sky, but never flat black.
SKY_LIGHT = (0.42, 0.52, 0.68)     # cool key from above
BOUNCE = (0.55, 0.30, 0.14)        # warm bounce from the land below
BACK = (0.046, 0.054, 0.048)
FLANK = (0.140, 0.145, 0.106)
BELLY = (0.255, 0.140, 0.076)
MEMB_IN = (0.038, 0.017, 0.015)
MEMB_OUT = (0.105, 0.038, 0.026)
VEIN = (0.048, 0.020, 0.018)
BONE = (0.070, 0.064, 0.056)
HORN = (0.520, 0.478, 0.385)
EYE = (2.20, 1.15, 0.20)
RIM = (1.00, 0.62, 0.30)


# ------------------------------------------------------------------- the dragon
def rot_about(pt, hinge, ang):
    """Rotate `pt` around `hinge`. Screen coords, so a positive angle swings a
    forward-pointing part downward — which is how a jaw opens."""
    dx, dy = pt[0] - hinge[0], pt[1] - hinge[1]
    c, s = math.cos(ang), math.sin(ang)
    return (hinge[0] + dx * c - dy * s, hinge[1] + dx * s + dy * c)


def dragon_parts(phase, jaw=0.0):
    """One frame's primitive list, plus the metadata a ROLL needs.

    `phase` is the wingbeat angle; `s` drives the stroke, +1 wings fully up and
    -1 fully down. The body bobs against the stroke and the tail trails it by a
    fixed lag — that lag is most of what makes the stills read as one animal
    rather than a row of poses.

    Returns `(parts, meta, spine)`. Every part gets a `meta` entry of
    `(z, rollf, wing)`:

      z      how far the part sits to the near (+) or far (-) SIDE of the body.
             Zero for anything on the midline; ±span for the two wings.
      rollf  how much of the body's roll this part follows. The head is on 0.55
             because a flying animal stabilises its head — rolling the skull
             with the torso is the single thing that most makes a roll look
             like a spinning model rather than a manoeuvre.
      wing   membranes keep a sliver of width when edge-on instead of vanishing.

    A roll then rotates each part around the spine in the (y, z) plane, which is
    what makes the wings sweep through edge-on and swap depth, the dorsal ridge
    roll under, and the legs come up — rather than the whole picture squashing.
    """
    s = math.sin(phase)
    bob = -0.016 * s
    parts = []
    meta = []

    def emit(part, z=0.0, rollf=1.0, wing=False):
        parts.append(part)
        meta.append((z, rollf, wing))

    def y(v):
        return v + bob

    # -- wings ---------------------------------------------------------------
    def wing(shoulder, hip, span, s_eff, style, layer, zside):
        # The arm points back and either up or down with the stroke; near mid
        # stroke it flattens, which reads as edge-on foreshortening.
        arm = norm((-0.52, -1.05 * s_eff))
        wrist = add(shoulder, arm, span)
        back = (-1.0, 0.0)
        # Fan the fingers from the arm TOWARD the tail, whichever side that is.
        cross = arm[0] * back[1] - arm[1] * back[0]
        sign = 1.0 if cross > 0.0 else -1.0
        tips = []
        for ang, ln in ((0.30, 0.78), (0.66, 0.70), (1.02, 0.585), (1.40, 0.44)):
            d = rot(arm, sign * ang)
            tips.append(add(wrist, d, span * ln))
        poly = [shoulder, wrist] + tips + [hip]
        emit(("memb", poly, tips, wrist, shoulder, style, layer),
             z=zside, wing=True)
        # The ARM is a real limb — muscled, shaded like the rest of the animal —
        # so it is a solid capsule along the leading edge. The FINGERS are not
        # drawn as separate strokes: they are ridges shaded INSIDE the membrane
        # (see `membrane_shade`), because a bone laid over the membrane as a
        # line is exactly what makes a wing read as a wireframe umbrella.
        emit(("cap", shoulder, wrist, span * 0.062, span * 0.034,
              "limb", layer + 0.1), z=zside)

    # Far wing: behind in the stroke and smaller, so the two never overlap into
    # one flat shape — the phase offset is what gives the frame depth.
    far_s = math.sin(phase - 0.42)
    wing((0.470, y(0.478)), (0.395, y(0.560)), 0.300, far_s, "memb_far", 0.0,
         -0.055)

    # -- tail, body, neck, head ----------------------------------------------
    sway = 0.055 * math.sin(phase - 1.15)
    spine = [
        ((0.040, y(0.415 + sway * 1.35)), 0.005),
        ((0.135, y(0.492 + sway * 0.95)), 0.017),
        ((0.245, y(0.540 + sway * 0.50)), 0.031),
        ((0.360, y(0.556)), 0.049),
        ((0.470, y(0.550)), 0.062),
        ((0.560, y(0.536)), 0.058),
        ((0.640, y(0.498)), 0.042),
        ((0.706, y(0.452)), 0.032),
        ((0.752, y(0.428)), 0.029),
    ]
    for i in range(len(spine) - 1):
        (a, ra), (b, rb) = spine[i], spine[i + 1]
        emit(("cap", a, b, ra, rb, "body", 1.0))

    # Dorsal ridge. On the midline, so a roll carries it around the body and it
    # ends up underneath at 180 degrees — which is most of how you read that a
    # rolled dragon is inverted.
    for i in range(7):
        u = i / 6.0
        bx = mix(0.205, 0.600, u)
        by = y(mix(0.515, 0.480, u)) - mix(0.026, 0.052, math.sin(u * math.pi))
        emit(("cap", (bx, by + 0.028), (bx - 0.013, by), 0.011, 0.002,
              "horn", 1.05))

    # Legs, tucked up under the body in flight. Slightly off the midline, so the
    # near pair passes in front of the body through a roll and the far pair
    # behind it.
    emit(("cap", (0.452, y(0.600)), (0.404, y(0.664)), 0.023, 0.015,
          "body", 1.1), z=0.022)
    emit(("cap", (0.404, y(0.664)), (0.470, y(0.694)), 0.015, 0.010,
          "body", 1.1), z=0.022)
    emit(("cap", (0.470, y(0.694)), (0.498, y(0.703)), 0.010, 0.003,
          "claw", 1.12), z=0.022)
    emit(("cap", (0.556, y(0.588)), (0.586, y(0.646)), 0.019, 0.012,
          "body", 1.1), z=0.026)
    emit(("cap", (0.586, y(0.646)), (0.640, y(0.660)), 0.012, 0.008,
          "body", 1.1), z=0.026)
    emit(("cap", (0.640, y(0.660)), (0.664, y(0.666)), 0.008, 0.003,
          "claw", 1.12), z=0.026)

    # Tail fin — a membrane, so it catches light like the wings do. Vertical on
    # the midline, so it too sweeps through edge-on as the animal rolls.
    emit(("poly", [(0.036, y(0.412 + sway * 1.35)),
                   (0.100, y(0.352 + sway * 1.2)),
                   (0.132, y(0.462 + sway * 1.0)),
                   (0.068, y(0.462 + sway * 1.15))],
          "memb_far", 1.15), wing=True)

    # Skull, snout, jaw. All on `rollf` 0.55: the head stays closer to level
    # than the torso, the way a flying animal actually holds it.
    emit(("ell", (0.778, y(0.424)), 0.052, 0.037, -0.22, "body", 1.2), rollf=0.55)
    emit(("cap", (0.790, y(0.432)), (0.872, y(0.456)), 0.023, 0.012,
          "body", 1.2), rollf=0.55)
    # The lower jaw swings about its hinge. With it open the maw behind it shows
    # as a dark throat lit from inside — which is what makes a breathing dragon
    # look like it is breathing rather than like fire happening near its face.
    hinge = (0.786, y(0.450))
    jaw_tip = rot_about((0.852, y(0.472)), hinge, 0.62 * jaw)
    if jaw > 0.04:
        emit(("poly", [(0.792, y(0.444)), (0.866, y(0.456)),
                       jaw_tip, hinge], "maw", 1.185), rollf=0.55)
    emit(("cap", hinge, jaw_tip, 0.013, 0.007, "body", 1.19), rollf=0.55)
    # Horns sweeping back off the skull, and a brow spike.
    # Two segments each, so the horns carry a slight backward curve instead of
    # reading as straight antennae.
    emit(("cap", (0.768, y(0.398)), (0.724, y(0.364)), 0.014, 0.008,
          "horn", 1.25), rollf=0.55)
    emit(("cap", (0.724, y(0.364)), (0.690, y(0.328)), 0.008, 0.0015,
          "horn", 1.25), rollf=0.55)
    emit(("cap", (0.762, y(0.412)), (0.726, y(0.388)), 0.010, 0.006,
          "horn", 1.25), rollf=0.55)
    emit(("cap", (0.726, y(0.388)), (0.694, y(0.370)), 0.006, 0.0015,
          "horn", 1.25), rollf=0.55)
    emit(("cap", (0.802, y(0.404)), (0.840, y(0.392)), 0.007, 0.002,
          "horn", 1.25), rollf=0.55)
    emit(("ell", (0.806, y(0.421)), 0.011, 0.0085, 0.0, "eye", 1.3),
         z=0.020, rollf=0.55)

    # Near wing, over everything.
    wing((0.545, y(0.500)), (0.430, y(0.566)), 0.345, s, "memb_near", 2.0,
         0.055)

    # NOT sorted here any more: with a roll in play, draw order is decided by
    # where each part ends up in depth, which `roll_parts` works out.
    return parts, meta, spine


def axis_y(spine, x):
    """Height of the body axis at `x` — the line a roll rotates everything about."""
    if x <= spine[0][0][0]:
        return spine[0][0][1]
    if x >= spine[-1][0][0]:
        return spine[-1][0][1]
    for i in range(len(spine) - 1):
        x0, y0 = spine[i][0]
        x1, y1 = spine[i + 1][0]
        if x0 <= x <= x1:
            return mix(y0, y1, (x - x0) / (x1 - x0 + 1e-9))
    return spine[-1][0][1]


def roll_parts(parts, meta, spine, theta):
    """Rotate every part about the body axis by `theta`, then order by depth.

    Each point is decomposed into (height above the axis, distance to the side)
    and rotated in that plane. A part directly above the spine swings to the
    side and then below; a wing out to one side swings up and toward the viewer.
    Depth order falls out of the rotated z, so the two wings genuinely trade
    places as the animal goes over — nothing is layered by hand.
    """
    out = []
    for part, (z, rollf, is_wing) in zip(parts, meta):
        th = theta * rollf
        ct = math.cos(th)
        st = math.sin(th)
        # A membrane seen exactly edge-on has no area at all. Real wings still
        # catch the light as a thin line, so keep a sliver rather than letting
        # them blink out entirely.
        # 0.05, not 0.10: a wing seen edge-on is nearly nothing. Leaving it wider
        # turns the membrane into a flat plank through the middle of the roll.
        cy = math.copysign(max(abs(ct), 0.05), ct) if is_wing else ct

        zs = []

        def tp(pt):
            ay = axis_y(spine, pt[0])
            yr = pt[1] - ay
            zs.append(yr * st + z * ct)
            return (pt[0], ay + yr * cy - z * st)

        kind = part[0]
        if kind == "cap":
            _, a, b, ra, rb, style, layer = part
            new = ("cap", tp(a), tp(b), ra, rb, style, layer)
        elif kind == "ell":
            _, c, rx, ry, rr, style, layer = part
            # An ellipse rolls about the axis like anything else, and flattens
            # as it goes over the top.
            nc = tp(c)
            new = ("ell", nc, rx, max(ry * max(abs(ct), 0.35), 0.004), rr,
                   style, layer)
        elif kind == "poly":
            _, pts, style, layer = part
            new = ("poly", [tp(q) for q in pts], style, layer)
        else:
            _, poly, tips, wrist, shoulder, style, layer = part
            new = ("memb", [tp(q) for q in poly], [tp(q) for q in tips],
                   tp(wrist), tp(shoulder), style, layer)

        depth = sum(zs) / max(len(zs), 1)
        # A membrane's shading keys off `memb_far`, so re-label it by where it
        # actually ended up rather than by which wing it started as.
        if kind == "memb":
            style = "memb_far" if depth < 0.0 else "memb_near"
            new = ("memb", new[1], new[2], new[3], new[4], style, new[6])
        out.append((depth, part[-1], new))

    out.sort(key=lambda e: (e[0], e[1]))
    return [e[2] for e in out]


def shade(kind, d, r, t, off, px, py, aa):
    """Colour + coverage for one solid primitive at one pixel.

    `core` is a thickness proxy — 1 at the axis of the shape, 0 at the
    silhouette. Everything about the form reads off it.
    """
    cov = clamp(0.5 - d / aa)
    if cov <= 0.0:
        return None
    core = clamp(-d / max(r, 1e-4))

    if kind == "eye":
        # Amber, with a vertical slit pupil and a hot inner glow.
        slit = smoothstep(0.0042, 0.0011, abs(off[0]))
        glow = 0.55 + 0.45 * core
        col = mix3(tuple(c * glow for c in EYE), (0.030, 0.012, 0.004),
                   slit * 0.88)
        return col, cov

    # Surface normal, approximated from the offset to the shape's axis: the
    # lateral component IS the normal for a tube, and that is enough to light it.
    ny = clamp(off[1] / max(r, 1e-4), -1.0, 1.0)
    facing = math.sqrt(max(0.0, 1.0 - ny * ny))       # 1 facing viewer, 0 at edge

    # Two-source lighting: cool skylight from above, warm bounce from below.
    key = clamp(0.5 - 0.5 * ny)                        # 1 on top, 0 underneath
    fill = clamp(0.5 + 0.5 * ny)
    lit = [SKY_LIGHT[i] * (0.22 + 0.78 * key ** 1.4) +
           BOUNCE[i] * 0.34 * (fill ** 1.6) for i in range(3)]

    if kind == "bone":
        alb = BONE
    elif kind == "horn":
        # Keratin: banded along its length, paler at the tip.
        band = 0.86 + 0.14 * math.sin(t * 46.0)
        alb = tuple(c * band * mix(0.75, 1.15, t) for c in HORN)
    elif kind == "claw":
        alb = tuple(c * mix(0.55, 1.05, t) for c in HORN)
    else:
        # Hide: back to flank to belly, by which way the surface faces.
        alb = mix3(BACK, FLANK, smoothstep(-0.55, 0.35, ny))
        alb = mix3(alb, BELLY, smoothstep(0.30, 0.95, ny))
        # Mottling, plus ventral scutes banding across the underside.
        mot = fbm(px * 26.0, py * 26.0, 4.0)
        alb = tuple(c * mix(0.80, 1.22, mot) for c in alb)
        scute = 0.5 + 0.5 * math.sin(t * 150.0)
        alb = tuple(alb[i] * mix(1.0, mix(0.80, 1.14, scute),
                                 smoothstep(0.35, 0.95, ny)) for i in range(3))

    col = [alb[i] * lit[i] * (0.55 + 0.45 * facing) for i in range(3)]

    # Specular sheen along the lit upper flank — wet-leather, not plastic.
    spec = (clamp(0.5 - 0.5 * (ny + 0.55)) ** 8.0) * 0.55
    for i in range(3):
        col[i] += SKY_LIGHT[i] * spec

    # Warm rim on the upper silhouette: the single strongest realism cue at
    # small on-screen sizes, because it separates the animal from the sky.
    rimw = smoothstep(0.40, 0.0, core) * smoothstep(0.35, -0.85, ny)
    for i in range(3):
        col[i] += RIM[i] * 0.46 * rimw

    return tuple(col), cov


def membrane_shade(d, inside, px, py, aa, far, vein, bone, root):
    """Wing membrane: thin, translucent, veined, and dark where it meets the body.

    Real wing membrane is lit mostly from BEHIND — it glows at the thin trailing
    edge and goes opaque and shadowed toward the arm. That gradient, plus the
    veins, is the whole effect.
    """
    cov = clamp(0.5 - d / aa)
    if cov <= 0.0:
        return None
    thin = 1.0 - inside                       # 1 at the trailing edge
    col = mix3(MEMB_IN, MEMB_OUT, thin ** 1.7)
    # Backlight bleeding through the thinnest part — a narrow band right at the
    # trailing edge, not a wash over the whole wing.
    glow = smoothstep(0.70, 1.0, thin)
    col = mix3(col, (0.85, 0.36, 0.16), glow * 0.46)
    # Mottling, so the membrane is skin rather than a smooth gradient.
    col = tuple(c * mix(0.84, 1.14, fbm(px * 22.0, py * 22.0, 33.0))
                for c in col)
    # Veins darken and are most visible where the membrane is translucent.
    col = mix3(col, VEIN, vein * mix(0.30, 0.72, thin))
    # Finger bones: ridges UNDER the skin, not lines on top of it. A bone blocks
    # the backlight (so it darkens) and stands proud of the surface (so its upper
    # side catches a highlight). Both cues, and no stroke anywhere.
    if bone > 0.0:
        col = mix3(col, (0.055, 0.030, 0.026), bone * 0.80)
        col = tuple(col[i] + (0.30, 0.22, 0.17)[i] * (bone ** 2.2) * 0.55
                    for i in range(3))
    # Ambient occlusion into the wing root.
    ao = mix(0.42, 1.0, smoothstep(0.0, 0.35, root))
    col = tuple(c * ao for c in col)
    alpha = mix(0.95, 0.62, thin ** 1.2)
    alpha = mix(alpha, 1.0, bone * 0.85)          # bone is opaque
    if far:
        # Atmospheric separation: the far wing is darker and cooler.
        col = (col[0] * 0.30, col[1] * 0.33, col[2] * 0.42)
        alpha *= 0.96
    return col, cov * alpha


def render_dragon_cell(buf, w, ox, oy, cell, phase, theta, jaw=0.0):
    parts, meta, spine = dragon_parts(phase, jaw)
    parts = roll_parts(parts, meta, spine, theta)
    aa = 1.5 / cell

    boxes = []
    for p in parts:
        if p[0] == "cap":
            _, a, b, ra, rb, _, _ = p
            m = max(ra, rb) + aa
            boxes.append((min(a[0], b[0]) - m, min(a[1], b[1]) - m,
                          max(a[0], b[0]) + m, max(a[1], b[1]) + m))
        elif p[0] == "ell":
            _, c, rx, ry, _, _, _ = p
            m = max(rx, ry) + aa
            boxes.append((c[0] - m, c[1] - m, c[0] + m, c[1] + m))
        else:
            pts = p[1]
            xs = [q[0] for q in pts]
            ys = [q[1] for q in pts]
            boxes.append((min(xs) - aa, min(ys) - aa, max(xs) + aa, max(ys) + aa))

    for py_i in range(cell):
        py = (py_i + 0.5) / cell
        row = (oy + py_i) * w
        for px_i in range(cell):
            px = (px_i + 0.5) / cell
            cr = cg = cb = 0.0
            ca = 0.0
            for pi, p in enumerate(parts):
                bx0, by0, bx1, by1 = boxes[pi]
                if px < bx0 or px > bx1 or py < by0 or py > by1:
                    continue
                kind = p[0]
                out = None
                if kind == "cap":
                    _, a, b, ra, rb, style, _ = p
                    d, r, t, off = sd_capsule(px, py, a, b, ra, rb)
                    out = shade(style, d, r, t, off, px, py, aa)
                elif kind == "ell":
                    _, c, rx, ry, rr, style, _ = p
                    d, r, t, off = sd_ellipse(px, py, c, rx, ry, rr)
                    out = shade(style, d, r, t, off, px, py, aa)
                elif kind == "poly":
                    d = sd_poly(px, py, p[1])
                    if p[2] == "maw":
                        cov = clamp(0.5 - d / aa)
                        out = None
                        if cov > 0.0:
                            # Dark at the lips, glowing further in: the fire is
                            # already lit down there before it leaves.
                            deep = clamp(-d / 0.026)
                            col = mix3((0.028, 0.010, 0.008),
                                       (1.35, 0.42, 0.09), deep ** 0.8)
                            out = (col, cov)
                    else:
                        out = membrane_shade(d, clamp(-d / 0.055), px, py, aa,
                                             True, 0.0, 0.0, 1.0)
                elif kind == "memb":
                    _, poly, tips, wrist, shoulder, style, _ = p
                    d = sd_poly(px, py, poly)
                    # Scallop the trailing edge between consecutive finger tips.
                    for k in range(len(tips) - 1):
                        a, b = tips[k], tips[k + 1]
                        mx, my = (a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5
                        ex, ey = b[0] - a[0], b[1] - a[1]
                        el = math.hypot(ex, ey) + 1e-9
                        nx, ny_ = -ey / el, ex / el
                        s0 = 1.0 if (nx * (mx - wrist[0]) +
                                     ny_ * (my - wrist[1])) > 0 else -1.0
                        # Shallow scallops. Cut deeper and the fingertips stand
                        # out past the skin like spider legs.
                        rad = el * 0.58
                        cxx = mx + nx * s0 * rad * 0.94
                        cyy = my + ny_ * s0 * rad * 0.94
                        cut = rad - math.hypot(px - cxx, py - cyy)
                        if cut > d:
                            d = cut
                    if d < aa * 0.5:
                        # Veins fan from the wrist toward each fingertip; the
                        # membrane between them is where the light comes through.
                        # Finger bones, tapering wrist -> tip, read as ridges
                        # beneath the membrane rather than as drawn lines.
                        bone = 0.0
                        for tp in tips:
                            dv, tv, _ = sd_seg(px, py, wrist[0], wrist[1],
                                               tp[0], tp[1])
                            bw = mix(0.0125, 0.0022, tv)
                            b = smoothstep(bw, bw * 0.30, dv)
                            if b > bone:
                                bone = b
                        # Fine crossveins between the fingers.
                        crs = fbm(px * 34.0, py * 34.0, 21.0)
                        vein = clamp(smoothstep(0.60, 0.82, crs) * 0.55)
                        droot, _, _ = sd_seg(px, py, shoulder[0], shoulder[1],
                                             wrist[0], wrist[1])
                        out = membrane_shade(d, clamp(-d / 0.200), px, py, aa,
                                             style == "memb_far", vein, bone,
                                             droot)
                    else:
                        out = None
                if out is None:
                    continue
                col, a_ = out
                if a_ <= 0.0:
                    continue
                na = a_ + ca * (1.0 - a_)
                if na <= 1e-6:
                    continue
                inv = ca * (1.0 - a_)
                cr = (col[0] * a_ + cr * inv) / na
                cg = (col[1] * a_ + cg * inv) / na
                cb = (col[2] * a_ + cb * inv) / na
                ca = na
            if ca <= 0.002:
                continue
            o = (row + ox + px_i) * 4
            buf[o] = srgb(cr)
            buf[o + 1] = srgb(cg)
            buf[o + 2] = srgb(cb)
            buf[o + 3] = int(round(clamp(ca) * 255))


def build_dragon_sheet(path):
    """A column per wingbeat frame, a row per roll angle.

    The two animations are independent — a dragon can be anywhere in its
    wingbeat at any roll — so they are separate axes of one atlas rather than
    one long strip.
    """
    cell = DRAGON_CELL
    cols = FLAP_STEPS * MOUTH_STEPS
    w, h = cols * cell, ROLL_STEPS * cell
    buf = bytearray(w * h * 4)
    for r in range(ROLL_STEPS):
        theta = 2.0 * math.pi * r / ROLL_STEPS
        for m in range(MOUTH_STEPS):
            jaw = float(m)
            for f in range(FLAP_STEPS):
                phase = 2.0 * math.pi * f / FLAP_STEPS
                col = m * FLAP_STEPS + f
                render_dragon_cell(buf, w, col * cell, r * cell, cell,
                                   phase, theta, jaw)
        print(f"    roll {r + 1}/{ROLL_STEPS}", flush=True)
    write_png(path, w, h, buf)


# -------------------------------------------------------------------- the flame
def build_flame_sheet(path):
    """16 frames of a puff of dragonfire: ignite, roll, break up, die.

    Fire has no silhouette — it is a density field. So the shape comes from
    noise pushed around a growing radius, and the colour from the density
    itself: smoky red at the fringe through orange to a white-hot core.
    """
    cell = FLAME_CELL
    w, h = COLS * cell, ROWS * cell
    buf = bytearray(w * h * 4)
    n = COLS * ROWS
    for f in range(n):
        t = f / (n - 1)
        ox, oy = (f % COLS) * cell, (f // COLS) * cell
        env = math.sin(math.pi * (t ** 0.62))          # ignite fast, die slow
        rise = 0.15 * t                                 # the puff lifts as it burns
        radius = 0.17 + 0.29 * (t ** 0.62)
        breakup = smoothstep(0.35, 1.0, t)              # late frames shred
        for yy in range(cell):
            v0 = (yy + 0.5) / cell * 2.0 - 1.0 + rise * 2.0
            for xx in range(cell):
                u = (xx + 0.5) / cell * 2.0 - 1.0
                v = v0
                r = math.hypot(u, v * 0.86)
                if r > 0.98:
                    continue
                ang = math.atan2(v, u)
                turb = fbm(math.cos(ang) * 2.1 + t * 1.7,
                           math.sin(ang) * 2.1 - t * 2.4, 3.0 + f * 0.11)
                edge = radius * (1.0 + 0.55 * (turb - 0.5) * (0.45 + breakup))
                body = smoothstep(edge, edge * 0.28, r)
                if body <= 0.002:
                    continue
                grain = fbm(u * 3.6 + t * 2.0, v * 3.6 - t * 3.1, 9.0 + f * 0.7)
                body *= mix(1.0, grain * 1.40, 0.45 * breakup)
                heat = clamp(body * env * 1.28)
                if heat <= 0.004:
                    continue
                col = mix3((0.62, 0.115, 0.030), (1.35, 0.50, 0.085),
                           smoothstep(0.10, 0.55, heat))
                col = mix3(col, (1.85, 1.35, 0.68), smoothstep(0.62, 0.96, heat))
                a = clamp(heat * 1.12)
                o = ((oy + yy) * w + ox + xx) * 4
                buf[o] = srgb(col[0])
                buf[o + 1] = srgb(col[1])
                buf[o + 2] = srgb(col[2])
                buf[o + 3] = int(round(a * 255))
        print(f"    frame {f + 1}/{n}", flush=True)
    write_png(path, w, h, buf)


def build_breath_sheet(path):
    """16 frames of a JET of dragonfire — the mouth at the left edge, the tip at
    the right. Wide cells, because a jet is not square.

    A round puff repeated along a line never reads as flame, however well the
    puff is drawn: fire in a jet is made of TONGUES — long filaments that lick
    downstream, separated by gaps you can see through. So the shape here is a
    cone carved by noise that is stretched along the jet and threshold-cut, and
    the gaps between the tongues carry alpha 0 rather than dark colour.

    Three things do the work:
      * the noise is anisotropic — much lower frequency along the jet than
        across it, which is what turns blobs into licks;
      * it is domain-warped by a second field that grows downstream, so the
        tongues curl instead of running straight;
      * the frames advect the field along the jet, so successive frames read as
        fire streaming outward rather than as a shape wobbling in place.
    """
    cols, rows = BREATH_COLS, BREATH_ROWS
    cw, ch = BREATH_W, BREATH_H
    w, h = cols * cw, rows * ch
    buf = bytearray(w * h * 4)
    n = cols * rows
    for f in range(n):
        t = f / n
        ox, oy = (f % cols) * cw, (f // cols) * ch
        for yy in range(ch):
            v0 = (yy + 0.5) / ch * 2.0 - 1.0
            for xx in range(cw):
                u = (xx + 0.5) / cw
                # The cone. Narrow at the throat, opening down the jet — but a
                # jet, not a fan: it stays much longer than it is wide.
                hw = 0.055 + 0.60 * (u ** 0.9)
                # Curl: displace across the jet by a field that grows with
                # distance from the mouth.
                warp = fbm(u * 4.0 - t * 5.0, v0 * 2.5 + 3.0, 7.0, 3)
                v = v0 + (warp - 0.5) * 0.55 * (u ** 0.85)
                r = abs(v) / hw
                if r > 1.7:
                    continue
                core = smoothstep(1.0, 0.10, r)
                if core <= 0.002:
                    continue
                # Tongues. The frequencies are deliberately ANISOTROPIC — low
                # along the jet, high across it — so the noise stretches into
                # long filaments instead of breaking into blobs. Isotropic noise
                # here is exactly what makes fire read as smoke.
                # TWO scales: long filaments, plus a rounder turbulence on top.
                # Filaments alone comb into smooth strands and read as molten
                # metal or hair; the second scale is what breaks them up.
                fil = fbm(u * 1.9 - t * 6.0, v * 10.0 + 11.0, 19.0, 4)
                chop = fbm(u * 5.5 - t * 8.0, v * 5.5 + 31.0, 41.0, 4)
                lick = fil * 0.60 + chop * 0.40
                # Cut hard, so the gaps between the licks open to nothing. Fire
                # you can see through between the tongues is most of the effect.
                tongue = smoothstep(0.42, 0.62, lick)
                dens = core * mix(0.06, 1.0, tongue)
                # The throat stays solid; the tip shreds into separate licks.
                bk = smoothstep(0.22, 1.0, u)
                dens = mix(dens, dens * tongue * 1.6, bk)
                dens += smoothstep(0.16, 0.0, u) * core * 0.9      # hot throat
                # Hottest at the mouth, cooling as it travels.
                heat = clamp(dens * mix(1.35, 0.30, u ** 0.65))
                if heat <= 0.03:
                    continue
                # Most of a flame is orange. White belongs only to the throat,
                # and the fringe should fall to deep red rather than to pale —
                # arriving at white early is what made this look like metal.
                col = mix3((0.42, 0.045, 0.012), (1.15, 0.30, 0.030),
                           smoothstep(0.05, 0.40, heat))
                col = mix3(col, (1.70, 0.85, 0.14), smoothstep(0.58, 0.82, heat))
                col = mix3(col, (2.20, 1.85, 1.30), smoothstep(0.91, 1.0, heat))
                a = clamp(heat * 1.25)
                o = ((oy + yy) * w + ox + xx) * 4
                buf[o] = srgb(col[0])
                buf[o + 1] = srgb(col[1])
                buf[o + 2] = srgb(col[2])
                buf[o + 3] = int(round(a * 255))
        print(f"    frame {f + 1}/{n}", flush=True)
    write_png(path, w, h, buf)


def main():
    os.makedirs(ART, exist_ok=True)
    print("breath sheet:")
    build_breath_sheet(os.path.join(ART, "breath.png"))
    print("flame sheet:")
    build_flame_sheet(os.path.join(ART, "flame.png"))
    print("dragon sheet:")
    build_dragon_sheet(os.path.join(ART, "dragon.png"))


if __name__ == "__main__":
    main()

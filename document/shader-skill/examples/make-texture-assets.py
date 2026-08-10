#!/usr/bin/env python3
"""Generate the PNG assets the tb-texture-* example bundles ship.

Kept as a script (and committed beside the bundles) so the art is reproducible
and reviewable as code rather than as an opaque binary blob.
"""
import math
import struct
import sys
import zlib


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
    print(f"  {path}  {w}x{h}")


def srgb(x):
    """linear 0..1 -> sRGB byte."""
    x = max(0.0, min(1.0, x))
    v = 12.92 * x if x <= 0.0031308 else 1.055 * (x ** (1 / 2.4)) - 0.055
    return int(round(v * 255))


# ---------------------------------------------------------------- sprite sheet
def spark_sheet(path, cols=4, rows=4, cell=64):
    """A 4x4 atlas of a spark igniting, burning and dying.

    Frame n is a soft radial core plus a ring of filaments, both scaled by an
    envelope over n so the 16 cells read as one animation rather than 16 blobs.
    Straight alpha, sRGB colour.
    """
    w, h = cols * cell, rows * cell
    buf = bytearray(w * h * 4)
    n = cols * rows
    for f in range(n):
        t = f / (n - 1)
        # Ignite fast, decay slow.
        env = math.sin(math.pi * (t ** 0.65))
        radius = 0.10 + 0.30 * (t ** 0.5)
        cx, cy = (f % cols) * cell, (f // cols) * cell
        for y in range(cell):
            for x in range(cell):
                u = (x + 0.5) / cell * 2 - 1
                v = (y + 0.5) / cell * 2 - 1
                d = math.hypot(u, v)
                ang = math.atan2(v, u)
                # Filaments: a low-order angular ripple that stretches as it burns.
                fil = 1.0 + 0.35 * math.sin(ang * 7 + t * 9.0) * (t ** 0.5)
                core = max(0.0, 1.0 - (d / (radius * fil)) ** 2)
                glow = math.exp(-(d / (radius * 2.2 * fil)) ** 2)
                a = min(1.0, env * (core * 1.0 + glow * 0.55))
                if a <= 0.004:
                    continue
                # White-hot centre through amber to a deep red rim.
                k = min(1.0, core * 1.6)
                r = 0.35 + 0.65 * k
                g = 0.05 + 0.62 * (k ** 1.8)
                b = 0.01 + 0.45 * (k ** 5.0)
                i = ((cy + y) * w + (cx + x)) * 4
                buf[i + 0] = srgb(r)
                buf[i + 1] = srgb(g)
                buf[i + 2] = srgb(b)
                buf[i + 3] = int(round(a * 255))
    write_png(path, w, h, buf)


# ------------------------------------------------------------------- LUT strip
def lut_strip(path, size=16):
    """A 16x16x16 colour LUT laid out as a horizontal strip of 16 slices.

    Slice z holds the (r, g) plane for blue = z/15. Sampled with `srgb: false` —
    these are coordinates, not colours, and linearising them would corrupt every
    entry. The grade itself: a cool teal lift in the shadows, a warm roll-off in
    the highlights, slight saturation boost. A neutral identity LUT would prove
    the wiring and show nothing.
    """
    w, h = size * size, size
    buf = bytearray(w * h * 4)
    for z in range(size):
        b0 = z / (size - 1)
        for y in range(size):
            g0 = y / (size - 1)
            for x in range(size):
                r0 = x / (size - 1)
                lum = 0.2126 * r0 + 0.7152 * g0 + 0.0722 * b0
                # Lift shadows toward teal, pull highlights toward amber.
                shadow = (1.0 - lum) ** 2
                high = lum ** 2
                r = r0 + 0.10 * high - 0.06 * shadow
                g = g0 + 0.05 * high + 0.02 * shadow
                b = b0 - 0.08 * high + 0.12 * shadow
                # Saturation, around the (shifted) luminance.
                m = 0.2126 * r + 0.7152 * g + 0.0722 * b
                s = 1.15
                r, g, b = m + (r - m) * s, m + (g - m) * s, m + (b - m) * s
                i = (y * w + (z * size + x)) * 4
                buf[i + 0] = max(0, min(255, int(round(r * 255))))
                buf[i + 1] = max(0, min(255, int(round(g * 255))))
                buf[i + 2] = max(0, min(255, int(round(b * 255))))
                buf[i + 3] = 255
    write_png(path, w, h, buf)


# ------------------------------------------------------------- tiling material
def paper(path, size=256, seed=12345):
    """A seamless paper grain: height in R, and its two slope components in G/B.

    Seamless because the noise is built from sine sums whose frequencies are
    whole numbers of cycles per tile, so the tile matches itself at every edge —
    which is what lets a shader repeat it with `fract()` without a visible seam.
    Sampled with `srgb: false`: R is a height and G/B are signed slopes biased to
    0.5, none of which are colours.
    """
    w = h = size
    buf = bytearray(w * h * 4)
    rng = seed

    def rnd():
        nonlocal rng
        rng = (rng * 1103515245 + 12345) & 0x7FFFFFFF
        return rng / 0x7FFFFFFF

    # A handful of integer-frequency waves at random phase/orientation.
    waves = []
    for k in range(28):
        fx = int(1 + rnd() * 12)
        fy = int(1 + rnd() * 12)
        if rnd() < 0.5:
            fx = -fx
        waves.append((fx, fy, rnd() * math.tau, 1.0 / (1 + abs(fx) + abs(fy))))
    norm = sum(a for _, _, _, a in waves)

    def height(u, v):
        s = 0.0
        for fx, fy, ph, a in waves:
            s += a * math.sin(math.tau * (fx * u + fy * v) + ph)
        return s / norm

    e = 1.0 / size
    for y in range(h):
        v = y / size
        for x in range(w):
            u = x / size
            c = height(u, v)
            dx = (height(u + e, v) - height(u - e, v)) * 0.5 / e
            dy = (height(u, v + e) - height(u, v - e)) * 0.5 / e
            i = (y * w + x) * 4
            buf[i + 0] = max(0, min(255, int(round((c * 0.5 + 0.5) * 255))))
            buf[i + 1] = max(0, min(255, int(round((dx * 0.06 + 0.5) * 255))))
            buf[i + 2] = max(0, min(255, int(round((dy * 0.06 + 0.5) * 255))))
            buf[i + 3] = 255
    write_png(path, w, h, buf)


if __name__ == "__main__":
    root = sys.argv[1]
    print("generating texture assets:")
    spark_sheet(f"{root}/tb-texture-sheet/sparks.png")
    lut_strip(f"{root}/tb-texture-grade/grade.png")
    paper(f"{root}/tb-texture-paper/paper.png")

    # `tb-texture-all` uses all three at once, and gets its OWN copies rather
    # than pointing at its siblings: a texture path may not contain `..`, unlike
    # a `modules` path, which shares WGSL between bundles on purpose. So the
    # duplication is the rule showing through, not an oversight — and generating
    # it here rather than checking in three more binaries keeps one source.
    import shutil
    print("  copies for tb-texture-all:")
    for name, src in [
        ("sparks.png", "tb-texture-sheet"),
        ("grade.png", "tb-texture-grade"),
        ("paper.png", "tb-texture-paper"),
    ]:
        shutil.copyfile(f"{root}/{src}/{name}", f"{root}/tb-texture-all/{name}")
        print(f"    {root}/tb-texture-all/{name}")

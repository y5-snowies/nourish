#!/usr/bin/env python3
"""Numeric read-out of a dmabuf-draw --grid screenshot.

No PIL: decodes 8-bit RGB/RGBA non-interlaced PNG with zlib + the standard
un-filter, then samples each cell at the geometry dmabuf-draw printed, so the
report is pixel values rather than an impression.

  ./inspect.py y5-capture-*.png [--cols=4] [--size=256x128] [--gap=4]
"""
import sys, zlib, struct

def load(path):
    data = open(path, 'rb').read()
    assert data[:8] == b'\x89PNG\r\n\x1a\n', "not a PNG"
    pos, idat, w = 8, b'', None
    while pos < len(data):
        ln, typ = struct.unpack('>I4s', data[pos:pos+8])
        body = data[pos+8:pos+8+ln]
        if typ == b'IHDR':
            w, h, depth, color, comp, filt, inter = struct.unpack('>IIBBBBB', body)
            assert depth == 8 and color in (2, 6) and inter == 0, (depth, color, inter)
            nch = 3 if color == 2 else 4
        elif typ == b'IDAT':
            idat += body
        elif typ == b'IEND':
            break
        pos += 12 + ln
    raw = zlib.decompress(idat)
    stride = w * nch
    out = bytearray(w * h * nch)
    prev = bytearray(stride)
    p = 0
    for y in range(h):
        f = raw[p]; p += 1
        line = bytearray(raw[p:p+stride]); p += stride
        if f == 1:
            for i in range(nch, stride): line[i] = (line[i] + line[i-nch]) & 0xff
        elif f == 2:
            for i in range(stride): line[i] = (line[i] + prev[i]) & 0xff
        elif f == 3:
            for i in range(stride):
                a = line[i-nch] if i >= nch else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 0xff
        elif f == 4:
            for i in range(stride):
                a = line[i-nch] if i >= nch else 0
                b = prev[i]
                c = prev[i-nch] if i >= nch else 0
                pa, pb, pc = abs(b-c), abs(a-c), abs(a+b-2*c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 0xff
        out[y*stride:(y+1)*stride] = line
        prev = line
    return w, h, nch, out

FORMATS = ["XR24","AR24","XB24","AB24","XR30","XB30","RG16","XR15",
           "BA12","RG24","R8","R16","XB4H","AB4H"]

def black_report(path):
    """Objective black-region report: any pure (0,0,0) pixel, and where.

    Use with `dmabuf-draw --pattern=check`, whose buffers contain NO black at all —
    so every black pixel found here was painted by the compositor, not the client."""
    w, h, nch, px = load(path)
    def at(x, y):
        i = (y*w + x)*nch
        return (px[i], px[i+1], px[i+2])
    total = 0
    x0, y0, x1, y1 = w, h, -1, -1
    rows = []
    for y in range(h):
        n = 0
        for x in range(w):
            if at(x, y) == (0, 0, 0):
                n += 1
                if x < x0: x0 = x
                if x > x1: x1 = x
        if n:
            total += n
            if y < y0: y0 = y
            if y > y1: y1 = y
            rows.append((y, n))
    print(f"image {w}x{h}: {total} pure-black pixel(s)"
          f" ({100.0*total/(w*h):.1f}% of the image)")
    if not total:
        print("  none — nothing was painted black")
        return
    print(f"  bounding box x {x0}..{x1}, y {y0}..{y1}")
    full = [y for y, n in rows if n >= w - 2]
    if full:
        print(f"  full-width black rows: {len(full)}, first {full[0]}, last {full[-1]}")
    runs, prev = [], None
    for y, n in rows:
        if prev is not None and y == prev[0] + prev[1] and n == prev[2]:
            prev[1] += 1
        else:
            prev = [y, 1, n]; runs.append(prev)
    print("  row runs (start, count, black px per row):")
    for r in runs[:12]:
        print(f"    y={r[0]:4d} x{r[1]:<4d} {r[2]} px")

def main():
    if sys.argv[1] == "--black":
        black_report(sys.argv[2]); return
    path = sys.argv[1]
    cols, TW, TH, gap = 4, 256, 128, 4
    for a in sys.argv[2:]:
        if a.startswith('--cols='): cols = int(a[7:])
        elif a.startswith('--size='): TW, TH = (int(v) for v in a[7:].split('x'))
        elif a.startswith('--gap='): gap = int(a[6:])
    w, h, nch, px = load(path)
    HDR = 26 if TH >= 96 else 16
    pitch_x, pitch_y = TW + gap, HDR + TH + gap
    def at(x, y):
        if not (0 <= x < w and 0 <= y < h): return None
        i = (y*w + x)*nch
        return (px[i], px[i+1], px[i+2])
    print(f"image {w}x{h} ({nch}ch) | cells {TW}x{HDR}+{TH} pitch {pitch_x}x{pitch_y}")
    print(f"backdrop sample (2,2) = {at(2,2)}   (expect ~ (48,48,48))\n")

    for i, code in enumerate(FORMATS):
        cx, cy = gap + (i % cols)*pitch_x, gap + (i // cols)*pitch_y
        by = cy + HDR                            # body top
        hue_y  = by + int(TH*0.25)               # inside the hue sweep
        ramp_y = by + int(TH*0.66)               # inside the grey ramp
        pat_y  = by + int(TH*0.92)               # inside the patch row
        hues  = [at(cx + int(TW*f), hue_y) for f in (0.02, 0.19, 0.35, 0.52, 0.68, 0.85)]
        ramps = [at(cx + int(TW*f), ramp_y) for f in (0.25, 0.50, 0.75)]
        pats  = [at(cx + int(TW*(0.125 + 0.25*k)), pat_y) for k in range(4)]
        blank = all(p == at(2, 2) for p in hues + ramps + pats)
        print(f"{code:5} cell({cx},{cy}) body({cx},{by})"
              f"{'   *** BODY MISSING (backdrop showing) ***' if blank else ''}")
        print(f"      hue   {hues}")
        print(f"      ramp  {ramps}   (expect ~(64,64,64) (128,128,128) (191,191,191))")
        print(f"      patch {pats}   (expect R G B W)")

main()

#!/usr/bin/env python3
"""Generate reIDM's application icon as PNG and a multi-size ICO.

No third-party modules: shapes are drawn from signed-distance fields, the PNG is
assembled by hand with zlib, and the ICO is a container around those PNGs. Run
from the repository root:

    python3 -I tools/make_icon.py

Writes into packaging/icon/: reidm-1024.png, reidm-512.png, reidm-256.png,
reidm-128.png, reidm-64.png, reidm-48.png, reidm-32.png, reidm-16.png and
reidm.ico (every size, so Explorer, the taskbar and the installer each pick a
crisp one instead of rescaling a single large bitmap).
"""
import math
import os
import struct
import zlib

OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "packaging", "icon")

# Palette: a deep, saturated blue that stays legible on both light and dark shells.
TOP = (0x3D, 0x9B, 0xFF)
BOTTOM = (0x0A, 0x46, 0xA0)
WHITE = (0xFF, 0xFF, 0xFF)

# Supersampling per axis. 4 gives clean curves without a drawing library.
SS = 4
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
PNG_SIZES = [1024, 512, 256, 128, 64, 48, 32, 24, 16]


def _len(x, y):
    return math.hypot(x, y)


def sd_rounded_box(px, py, cx, cy, hw, hh, r):
    """Signed distance to an axis-aligned rounded box (negative inside)."""
    dx = abs(px - cx) - (hw - r)
    dy = abs(py - cy) - (hh - r)
    outside = _len(max(dx, 0.0), max(dy, 0.0))
    inside = min(max(dx, dy), 0.0)
    return outside + inside - r


def _dist_to_segment(px, py, a, b):
    ax, ay = a
    bx, by = b
    vx, vy = bx - ax, by - ay
    wx, wy = px - ax, py - ay
    t = max(0.0, min(1.0, (wx * vx + wy * vy) / (vx * vx + vy * vy)))
    return _len(wx - vx * t, wy - vy * t)


def sd_triangle(px, py, a, b, c):
    """Signed distance to a triangle (negative inside).

    The side is decided by a winding-independent inside test: a point is inside when
    the three edge cross products all share a sign. An earlier attempt used the
    orientation trick from the usual analytic form and got the winding backwards,
    which filled the whole canvas with the glyph.
    """
    d = min(
        _dist_to_segment(px, py, a, b),
        _dist_to_segment(px, py, b, c),
        _dist_to_segment(px, py, c, a),
    )

    def cross(o, p, q):
        return (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])

    p = (px, py)
    d1 = cross(a, b, p)
    d2 = cross(b, c, p)
    d3 = cross(c, a, p)
    inside = not ((d1 < 0 or d2 < 0 or d3 < 0) and (d1 > 0 or d2 > 0 or d3 > 0))
    return -d if inside else d


def glyph(px, py, s):
    """The download mark: a bold arrow dropping onto a tray, in a square of side `s`."""
    u = s / 512.0
    cx = s / 2.0
    r_head = 26.0 * u
    r_tray = 26.0 * u
    r_shaft = 34.0 * u

    # Shaft, overlapping the head so the two read as one solid arrow.
    d = sd_rounded_box(px, py, cx, 226.0 * u, r_shaft, 118.0 * u, 30.0 * u)

    # Head: a wide triangle with rounded corners.
    head = sd_triangle(
        px, py,
        (cx - 122.0 * u, 268.0 * u),
        (cx + 122.0 * u, 268.0 * u),
        (cx, 398.0 * u),
    ) - r_head
    d = min(d, head)

    # Tray. The gap to the arrow tip is kept generous: at 16px a tighter one would
    # merge into a single blob.
    tray = sd_rounded_box(px, py, cx, 454.0 * u, 132.0 * u, 24.0 * u, r_tray)
    d = min(d, tray)
    return d


def render(size):
    """RGBA rows for one square icon."""
    rows = []
    r = size * 0.235
    n = SS * SS
    for py in range(size):
        row = bytearray()
        for px in range(size):
            bg_cov = 0.0
            fg_cov = 0.0
            for sy in range(SS):
                for sx in range(SS):
                    x = px + (sx + 0.5) / SS
                    y = py + (sy + 0.5) / SS
                    if sd_rounded_box(x, y, size / 2.0, size / 2.0, size / 2.0, size / 2.0, r) <= 0.0:
                        bg_cov += 1.0
                        if glyph(x, y, size) <= 0.0:
                            fg_cov += 1.0
            bg_cov /= n
            fg_cov /= n
            if bg_cov <= 0.0:
                row += b"\x00\x00\x00\x00"
                continue

            # Base: vertical gradient, with a soft highlight across the top third so the
            # shape reads as a physical key rather than a flat rectangle.
            t = py / max(size - 1, 1)
            base = [TOP[i] + (BOTTOM[i] - TOP[i]) * t for i in range(3)]
            gleam = 0.22 * max(0.0, 1.0 - abs(t - 0.16) / 0.28)
            base = [c + (255 - c) * gleam for c in base]

            k = fg_cov / bg_cov
            rgb = [round(base[i] + (WHITE[i] - base[i]) * k) for i in range(3)]
            row += bytes(rgb) + bytes([round(bg_cov * 255)])
        rows.append(bytes(row))
    return rows


def write_png(path, size, rows):
    raw = b"".join(b"\x00" + r for r in rows)

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    return png


def write_ico(path, entries):
    """entries: [(size, png_bytes)]. ICO carries PNG payloads, which every modern Windows reads."""
    count = len(entries)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    dir_entries = b""
    blobs = b""
    for size, png in entries:
        dim = 0 if size >= 256 else size
        dir_entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(png), offset)
        offset += len(png)
        blobs += png
    with open(path, "wb") as f:
        f.write(header + dir_entries + blobs)


def main():
    os.makedirs(OUT, exist_ok=True)
    pngs = {}
    for s in PNG_SIZES:
        data = write_png(os.path.join(OUT, f"reidm-{s}.png"), s, render(s))
        pngs[s] = data
        print(f"wrote {os.path.join(OUT, f'reidm-{s}.png')}")
    write_ico(os.path.join(OUT, "reidm.ico"), [(s, pngs[s]) for s in ICO_SIZES])
    print(f"wrote {os.path.join(OUT, 'reidm.ico')} (sizes: {', '.join(map(str, ICO_SIZES))})")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Generate reIDM's application icon as PNG and ICO.

No third-party modules: the PNG is assembled by hand with zlib, and the ICO is a thin container
around that PNG. Run from the repository root:

    python3 -I tools/make_icon.py

Writes packaging/icon/reidm-512.png, reidm-256.png and reidm.ico.
"""
import os
import struct
import zlib

OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "packaging", "icon")

# Accent colours, matching the desktop app.
TOP = (0x2A, 0x84, 0xE0)
BOTTOM = (0x08, 0x4A, 0xA8)
WHITE = (0xFF, 0xFF, 0xFF)
SS = 3  # supersampling factor per axis


def rounded_rect_coverage(x, y, w, h, r):
    """Coverage of a rounded rectangle whose edge is at distance `d`; 1 inside, 0 outside."""
    # Signed distance to a rounded rect centred in the canvas.
    cx, cy = w / 2.0, h / 2.0
    dx = abs(x - cx) - (w / 2.0 - r)
    dy = abs(y - cy) - (h / 2.0 - r)
    outside = ((max(dx, 0.0)) ** 2 + (max(dy, 0.0)) ** 2) ** 0.5
    inside = min(max(dx, dy), 0.0)
    return inside + outside - r


def arrow_coverage(x, y, w, h):
    """The download glyph: a shaft, a triangular head, and a tray line."""
    u = w / 512.0  # scale so the design is resolution independent
    cx = w / 2.0

    shaft_half = 30.0 * u
    shaft_top = 130.0 * u
    head_top = 250.0 * u
    tip_y = 370.0 * u
    head_half = 105.0 * u

    def sdf_rect(px, py, x0, y0, x1, y1):
        dx = max(x0 - px, px - x1)
        dy = max(y0 - py, py - y1)
        return ((max(dx, 0.0)) ** 2 + (max(dy, 0.0)) ** 2) ** 0.5 + min(max(dx, dy), 0.0)

    d = sdf_rect(x, y, cx - shaft_half, shaft_top, cx + shaft_half, head_top)

    # Triangle: signed distance to the two slanted edges plus the top edge.
    if head_top <= y <= tip_y:
        t = (y - head_top) / (tip_y - head_top)
        half = head_half * (1.0 - t)
        d = min(d, abs(x - cx) - half)
    else:
        d = min(d, 1e9)

    tray_y0 = 400.0 * u
    tray_y1 = 432.0 * u
    tray_half = 130.0 * u
    d = min(d, sdf_rect(x, y, cx - tray_half, tray_y0, cx + tray_half, tray_y1))
    return d


def render(size):
    """Return RGBA rows for one square icon of the given size."""
    rows = []
    r = size * 0.22
    for py in range(size):
        row = bytearray()
        for px in range(size):
            # Supersample for smooth edges without a font or drawing library.
            bg_cov = 0.0
            fg_cov = 0.0
            for sy in range(SS):
                for sx in range(SS):
                    x = px + (sx + 0.5) / SS
                    y = py + (sy + 0.5) / SS
                    if rounded_rect_coverage(x, y, size, size, r) <= 0.0:
                        bg_cov += 1.0
                        if arrow_coverage(x, y, size, size) <= 0.0:
                            fg_cov += 1.0
            n = SS * SS
            bg_cov /= n
            fg_cov /= n
            if bg_cov <= 0.0:
                row += b"\x00\x00\x00\x00"
                continue
            t = py / max(size - 1, 1)
            base = tuple(round(TOP[i] + (BOTTOM[i] - TOP[i]) * t) for i in range(3))
            k = fg_cov / bg_cov if bg_cov > 0 else 0.0
            rgb = tuple(round(base[i] + (WHITE[i] - base[i]) * k) for i in range(3))
            row += bytes(rgb) + bytes([round(bg_cov * 255)])
        rows.append(bytes(row))
    return rows


def write_png(path, size, rows):
    raw = b"".join(b"\x00" + r for r in rows)

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    return png


def write_ico(path, png_bytes, size):
    """An ICO may carry a PNG payload directly for entries of 256px and up."""
    header = struct.pack("<HHH", 0, 1, 1)
    entry = struct.pack("<BBBBHHII", 0, 0, 0, 0, 1, 32, len(png_bytes), 22)
    with open(path, "wb") as f:
        f.write(header + entry + png_bytes)


def main():
    os.makedirs(OUT, exist_ok=True)
    png512 = write_png(os.path.join(OUT, "reidm-512.png"), 512, render(512))
    print("wrote", os.path.join(OUT, "reidm-512.png"))
    write_png(os.path.join(OUT, "reidm-256.png"), 256, render(256))
    print("wrote", os.path.join(OUT, "reidm-256.png"))
    write_ico(os.path.join(OUT, "reidm.ico"), png512, 512)
    print("wrote", os.path.join(OUT, "reidm.ico"))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Generate reIDM's icon: a download button in front of a globe, with a small "re".

The globe and the download arrow are a nod to the classic download-manager icon reIDM grew out
of; the "re" says which one this is.

No third-party modules: every shape is a signed-distance field, edges are anti-aliased by
supersampling only the pixels an edge actually crosses, the PNG is assembled by hand with zlib,
and the ICO is a container around those PNGs. Run from the repository root:

    python3 -I tools/make_icon.py

Writes packaging/icon/reidm-{1024,512,256,128,64,48,32,24,16}.png, packaging/icon/reidm.ico
(every size, so Explorer, the taskbar and the installer each pick a crisp one), and the browser
extension's extension/icons/{16,32,48,128}.png.

Small sizes are drawn differently, not just scaled: below 48 px the letters would be a smudge,
so they are left out, the globe keeps only its equator and central meridian, and the button
grows so the arrow stays the thing you see.
"""
import math
import os
import struct
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "packaging", "icon")
EXT_OUT = os.path.join(ROOT, "extension", "icons")

GLOBE_LIGHT = (0x5C, 0xB8, 0xFF)
GLOBE_DARK = (0x0B, 0x3E, 0x96)
BUTTON_TOP = (0x3B, 0xD6, 0x6A)
BUTTON_BOTTOM = (0x12, 0x8A, 0x3E)
WHITE = (0xFF, 0xFF, 0xFF)

SS = 4  # supersampling per axis, for edge pixels only
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
PNG_SIZES = [1024, 512, 256, 128, 64, 48, 32, 24, 16]
EXT_SIZES = [16, 32, 48, 128]


def clamp(v, lo=0.0, hi=1.0):
    return lo if v < lo else hi if v > hi else v


def mix(a, b, t):
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))


def sd_rounded_box(px, py, cx, cy, hw, hh, r):
    """Signed distance to an axis-aligned rounded box (negative inside)."""
    dx = abs(px - cx) - (hw - r)
    dy = abs(py - cy) - (hh - r)
    outside = math.hypot(max(dx, 0.0), max(dy, 0.0))
    inside = min(max(dx, dy), 0.0)
    return outside + inside - r


def dist_segment(px, py, ax, ay, bx, by):
    vx, vy = bx - ax, by - ay
    wx, wy = px - ax, py - ay
    t = clamp((wx * vx + wy * vy) / (vx * vx + vy * vy))
    return math.hypot(wx - vx * t, wy - vy * t)


def sd_triangle(px, py, a, b, c):
    """Signed distance to a triangle (negative inside), winding-independent."""
    d = min(dist_segment(px, py, *a, *b), dist_segment(px, py, *b, *c), dist_segment(px, py, *c, *a))

    def cross(o, p, q):
        return (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0])

    p = (px, py)
    d1, d2, d3 = cross(a, b, p), cross(b, c, p), cross(c, a, p)
    inside = not ((d1 < 0 or d2 < 0 or d3 < 0) and (d1 > 0 or d2 > 0 or d3 > 0))
    return -d if inside else d


def dist_arc(px, py, cx, cy, r, a0, a1):
    """Distance to a circular arc from angle a0 to a1 (radians, increasing; screen y points
    down, so increasing angles turn clockwise on screen)."""
    ang = math.atan2(py - cy, px - cx)
    # Bring the angle into [a0, a0 + 2pi).
    while ang < a0:
        ang += 2 * math.pi
    while ang >= a0 + 2 * math.pi:
        ang -= 2 * math.pi
    if ang <= a1:
        return abs(math.hypot(px - cx, py - cy) - r)
    e0 = (cx + r * math.cos(a0), cy + r * math.sin(a0))
    e1 = (cx + r * math.cos(a1), cy + r * math.sin(a1))
    return min(math.hypot(px - e0[0], py - e0[1]), math.hypot(px - e1[0], py - e1[1]))


# ── the parts, in a 512-unit square ──────────────────────────────────────


def layout(size):
    """Where everything goes. Small icons give the button more room."""
    if size <= 24:
        # The arrow is the whole message at this size: a big button, a bold mark, a plain globe.
        return dict(gx=196, gy=196, gr=190, bx=336, by=336, bh=172, br=52, gap=34, letters=False, grid="none", line=0, bold=True)
    if size <= 32:
        return dict(gx=206, gy=206, gr=196, bx=354, by=354, bh=146, br=48, gap=30, letters=False, grid="simple", line=22)
    if size <= 64:
        return dict(gx=214, gy=214, gr=200, bx=358, by=358, bh=134, br=44, gap=22, letters=False, grid="full", line=14)
    return dict(gx=220, gy=220, gr=206, bx=362, by=362, bh=128, br=42, gap=18, letters=True, grid="full", line=9)


def arrow_sd(x, y, L):
    """The download mark — an arrow dropping onto a tray — inside the button."""
    # Map the button to a 512 box with padding, so the mark sits comfortably inside it.
    bold = L.get("bold", False)
    k = (L["bh"] * (0.86 if bold else 0.74)) / 256.0
    lx = (x - L["bx"]) / k + 256.0
    ly = (y - L["by"]) / k + 256.0
    w = 62.0 if bold else 40.0
    shaft = sd_rounded_box(lx, ly, 256.0, 214.0, w, 112.0, 34.0)
    head = sd_triangle(lx, ly, (256.0 - 150.0, 252.0), (256.0 + 150.0, 252.0), (256.0, 400.0)) - (34.0 if bold else 26.0)
    tray = sd_rounded_box(lx, ly, 256.0, 460.0, 150.0, 38.0 if bold else 28.0, 28.0)
    return min(shaft, head, tray) * k


def letters_sd(x, y, L):
    """A small lower-case "re" on the globe's upper left, drawn as strokes."""
    # Letter coordinates: x-height 64 units, stroke 14; placed and scaled into the globe.
    s = 1.06
    ox, oy = L["gx"] - 150.0, L["gy"] - 92.0
    lx, ly = (x - ox) / s, (y - oy) / s
    half = 7.5
    # r: a stem and a shoulder curving up and over to the right.
    r_stem = dist_segment(lx, ly, 8.0, 6.0, 8.0, 64.0)
    r_arm = dist_arc(lx, ly, 34.0, 30.0, 26.0, math.pi, math.pi * 1.80)
    # e: a bowl open at the lower right, and its bar.
    ecx, ecy, er = 92.0, 33.0, 29.0
    e_bowl = dist_arc(lx, ly, ecx, ecy, er, math.radians(30.0), math.radians(360.0))
    e_bar = dist_segment(lx, ly, ecx - er, ecy, ecx + er, ecy)
    return (min(r_stem, r_arm, e_bowl, e_bar) - half) * s


def grid_sd(x, y, L):
    """Latitude and longitude lines: straight parallels and elliptical meridians, as a globe
    seen from the front."""
    gx, gy, gr = L["gx"], L["gy"], L["gr"]
    nx, ny = (x - gx) / gr, (y - gy) / gr
    half = L["line"] / 2.0
    lats = [0.0] if L["grid"] == "simple" else [-0.5, 0.0, 0.5]
    meridians = [0.0] if L["grid"] == "simple" else [0.0, 0.52, 0.87]
    d = min(abs(ny - c) * gr for c in lats)
    w = math.sqrt(max(0.0, 1.0 - ny * ny))
    for a in meridians:
        d = min(d, abs(abs(nx) - a * w) * gr)
    return d - half


def shade(x, y, L, px_units):
    """Colour at one point (512 units), and how close the nearest edge is."""
    edges = []
    d_btn = sd_rounded_box(x, y, L["bx"], L["by"], L["bh"], L["bh"], L["br"])
    edges.append(d_btn)
    if d_btn <= 0.0:
        t = clamp((y - (L["by"] - L["bh"])) / (2.0 * L["bh"]))
        col = mix(BUTTON_TOP, BUTTON_BOTTOM, t)
        # A soft sheen along the top edge reads as a pressable button.
        col = mix(col, WHITE, 0.18 * clamp(1.0 - t / 0.35))
        a = arrow_sd(x, y, L)
        edges.append(a)
        if a <= 0.0:
            col = WHITE
        return col, 1.0, min(abs(e) for e in edges)

    # A transparent gap around the button lifts it off the globe.
    d_gap = d_btn - L["gap"]
    edges.append(d_gap)
    d_globe = math.hypot(x - L["gx"], y - L["gy"]) - L["gr"]
    edges.append(d_globe)
    if d_gap <= 0.0 or d_globe > 0.0:
        return (0.0, 0.0, 0.0), 0.0, min(abs(e) for e in edges)

    gx, gy, gr = L["gx"], L["gy"], L["gr"]
    t = clamp(((x - gx) * 0.55 + (y - gy) * 0.85) / gr * 0.5 + 0.5)
    col = mix(GLOBE_LIGHT, GLOBE_DARK, t)
    hl = clamp(1.0 - math.hypot(x - (gx - 0.42 * gr), y - (gy - 0.48 * gr)) / (0.78 * gr))
    col = mix(col, WHITE, 0.30 * hl * hl)

    if L["grid"] != "none":
        g = grid_sd(x, y, L)
        edges.append(g)
        if g <= 0.0:
            col = mix(col, WHITE, 0.30 if L["grid"] == "full" else 0.45)

    if L["letters"]:
        le = letters_sd(x, y, L)
        edges.append(le)
        if le <= 0.0:
            col = WHITE
    return col, 1.0, min(abs(e) for e in edges)


def render(size):
    """RGBA rows for one square icon."""
    L = layout(size)
    unit = 512.0 / size  # 512-units per pixel
    rows = []
    for py in range(size):
        row = bytearray()
        for px in range(size):
            cx, cy = (px + 0.5) * unit, (py + 0.5) * unit
            col, alpha, edge = shade(cx, cy, L, unit)
            if edge > unit * 0.75:
                acc = [c * alpha for c in col] + [alpha]
            else:
                acc = [0.0, 0.0, 0.0, 0.0]
                for sy in range(SS):
                    for sx in range(SS):
                        x = (px + (sx + 0.5) / SS) * unit
                        y = (py + (sy + 0.5) / SS) * unit
                        c, a, _ = shade(x, y, L, unit)
                        for i in range(3):
                            acc[i] += c[i] * a
                        acc[3] += a
                acc = [v / (SS * SS) for v in acc]
            a = acc[3]
            if a <= 0.0:
                row += b"\x00\x00\x00\x00"
                continue
            rgb = [round(clamp(acc[i] / a / 255.0) * 255.0) for i in range(3)]
            row += bytes(rgb) + bytes([round(clamp(a) * 255.0)])
        rows.append(bytes(row))
    return rows


def png_bytes(size, rows):
    raw = b"".join(b"\x00" + r for r in rows)

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
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
    os.makedirs(EXT_OUT, exist_ok=True)
    pngs = {}
    for s in sorted(set(PNG_SIZES + EXT_SIZES), reverse=True):
        pngs[s] = png_bytes(s, render(s))
        if s in PNG_SIZES:
            p = os.path.join(OUT, f"reidm-{s}.png")
            with open(p, "wb") as f:
                f.write(pngs[s])
            print(f"wrote {p}")
        if s in EXT_SIZES:
            p = os.path.join(EXT_OUT, f"{s}.png")
            with open(p, "wb") as f:
                f.write(pngs[s])
            print(f"wrote {p}")
    write_ico(os.path.join(OUT, "reidm.ico"), [(s, pngs[s]) for s in ICO_SIZES])
    print(f"wrote {os.path.join(OUT, 'reidm.ico')} (sizes: {', '.join(map(str, ICO_SIZES))})")


if __name__ == "__main__":
    main()

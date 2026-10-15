#!/usr/bin/env python3
"""Draw the background of the macOS disk image's window: a title, an arrow from where Finder
shows reIDM.app to where it shows the Applications folder, and a line on what happens next.

Finder places the two icons itself, at the positions `packaging/macos/dmg-settings.py` gives
(APP and APPS below must match them), and writes their names under them; this only draws what
sits behind. Needs Pillow and the Open Sans fonts (Debian/Ubuntu: fonts-open-sans):

    python3 -I tools/make_dmg_background.py

Writes packaging/macos/background.png (660x400) and background@2x.png (1320x800);
make-dmg.sh joins them into one image so Retina screens get the sharp one.
"""
import os
import sys

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "packaging", "macos")
FONTS = os.environ.get("FONT_DIR", "/usr/share/fonts/truetype/open-sans")

W, H = 660, 400
APP, APPS = (180, 196), (480, 196)  # icon centres, in points

TOP, BOTTOM = (0xF8, 0xFB, 0xFF), (0xE4, 0xEE, 0xFA)
INK, SOFT = (0x10, 0x27, 0x4A), (0x52, 0x63, 0x7A)
GREEN_A, GREEN_B = (0x3B, 0xD6, 0x6A), (0x12, 0x8A, 0x3E)  # the icon's button


def font(name, size):
    try:
        return ImageFont.truetype(os.path.join(FONTS, name), size)
    except OSError:
        sys.exit(f"missing font {name} in {FONTS} (set FONT_DIR)")


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def draw(scale):
    s = lambda v: round(v * scale)  # noqa: E731
    img = Image.new("RGB", (s(W), s(H)))
    px = ImageDraw.Draw(img)
    for y in range(s(H)):
        px.line([(0, y), (s(W), y)], fill=lerp(TOP, BOTTOM, y / (s(H) - 1)))

    # A soft halo under each icon, so the two places read as "from" and "to".
    halo = Image.new("L", img.size, 0)
    hd = ImageDraw.Draw(halo)
    for cx, cy in (APP, APPS):
        hd.rounded_rectangle([s(cx - 84), s(cy - 80), s(cx + 84), s(cy + 96)], radius=s(22), fill=150)
    img.paste((255, 255, 255), mask=halo)

    d = ImageDraw.Draw(img)
    d.text((s(W / 2), s(44)), "Install reIDM", font=font("OpenSans-Semibold.ttf", s(24)), fill=INK, anchor="mm")
    d.text((s(W / 2), s(74)), "Drag reIDM onto the Applications folder", font=font("OpenSans-Regular.ttf", s(14)), fill=SOFT, anchor="mm")

    # The arrow: a shaft with a rounded tail and a solid head, shaded like the icon's button.
    x0, x1, y = APP[0] + 92, APPS[0] - 92, APP[1] - 6
    head, thick = 22, 12
    arrow = Image.new("L", img.size, 0)
    ad = ImageDraw.Draw(arrow)
    ad.rounded_rectangle([s(x0), s(y - thick / 2), s(x1 - head + 2), s(y + thick / 2)], radius=s(thick / 2), fill=255)
    ad.polygon([(s(x1 - head), s(y - 19)), (s(x1), s(y)), (s(x1 - head), s(y + 19))], fill=255)
    shade = Image.new("RGB", img.size)
    sd = ImageDraw.Draw(shade)
    for yy in range(s(y - 20), s(y + 21)):
        sd.line([(0, yy), (s(W), yy)], fill=lerp(GREEN_A, GREEN_B, (yy - s(y - 20)) / max(1, s(40))))
    img.paste(shade, mask=arrow)

    d.text((s(W / 2), s(352)), "Then open reIDM, and add it to your browser from Help › Browser integration.",
           font=font("OpenSans-Regular.ttf", s(13)), fill=SOFT, anchor="mm")
    return img


if __name__ == "__main__":
    draw(1).save(os.path.join(OUT, "background.png"), optimize=True)
    draw(2).save(os.path.join(OUT, "background@2x.png"), optimize=True)
    print(f"wrote {OUT}/background.png and background@2x.png")

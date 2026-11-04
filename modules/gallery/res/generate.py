#!/usr/bin/env python3
"""Generate the gallery's image resources: small procedural PNGs, nothing downloaded.

Every image is a deterministic function of its pixel coordinates, drawn into a
palette-mode PNG so each stays well under 20 KB. Run from anywhere:

    python3 modules/gallery/res/generate.py

The committed files are the source of truth; this script documents how they
were made and regenerates them byte for byte with the same Pillow/zlib.
"""
import math
import os

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))

# (file, width, height, palette, function(x, y, w, h) -> palette index)
IMAGES = []


def image(name, w, h, palette):
    def register(fn):
        IMAGES.append((name, w, h, palette, fn))
        return fn

    return register


@image("01-ringar.png", 320, 200, [(0x1E, 0x3A, 0x8A), (0xF8, 0xFA, 0xFC), (0x60, 0xA5, 0xFA)])
def ringar(x, y, w, h):
    d = math.hypot(x - w / 2, y - h / 2)
    return int(d / 14) % 3


@image("02-rutor.png", 240, 240, [(0xFA, 0xCC, 0x15), (0x1F, 0x29, 0x37), (0xE5, 0xE7, 0xEB)])
def rutor(x, y, w, h):
    cell = 30
    cx, cy = x // cell, y // cell
    if (cx + cy) % 2 == 0:
        return 0
    return 1 if (cx * 7 + cy * 3) % 5 else 2


@image("03-vagor.png", 200, 300, [(0x0F, 0x76, 0x6E), (0x5E, 0xEA, 0xD4), (0xCC, 0xFB, 0xF1)])
def vagor(x, y, w, h):
    v = y + 18 * math.sin(x / 17.0) + 9 * math.sin(y / 23.0)
    return int(v / 16) % 3


@image("04-diagonal.png", 320, 180, [(0x9A, 0x34, 0x12), (0xFB, 0x92, 0x3C), (0xFF, 0xED, 0xD5), (0xC2, 0x41, 0x0C)])
def diagonal(x, y, w, h):
    band = int((x + y) / 20) % 4
    if (x - y) % 60 < 3:
        return 2
    return band


@image("05-prickar.png", 256, 256, [(0xFD, 0xF2, 0xF8), (0xBE, 0x18, 0x5D), (0xF9, 0xA8, 0xD4)])
def prickar(x, y, w, h):
    cell = 32
    cx, cy = x % cell - cell / 2 + 0.5, y % cell - cell / 2 + 0.5
    r = 6 + 6 * ((x // cell + y // cell) % 3)
    d = math.hypot(cx, cy)
    if d < r - 3:
        return 1
    if d < r:
        return 2
    return 0


@image("06-spiral.png", 300, 200, [(0x14, 0x53, 0x2D), (0x86, 0xEF, 0xAC), (0xDC, 0xFC, 0xE7)])
def spiral(x, y, w, h):
    dx, dy = x - w / 2 + 0.5, y - h / 2 + 0.5
    a = math.atan2(dy, dx)
    d = math.hypot(dx, dy)
    return int((a / (2 * math.pi) * 3 + d / 22)) % 3


@image("07-rutnat.png", 180, 320, [(0xF5, 0xF5, 0xF4), (0x44, 0x40, 0x3C), (0xA8, 0xA2, 0x9E)])
def rutnat(x, y, w, h):
    if x % 20 == 0 or y % 20 == 0:
        return 1
    if (x // 20 + y // 20) % 7 == 0:
        return 2
    return 0


@image("08-solnedgang.png", 320, 200, [(0x7C, 0x2D, 0x12), (0xEA, 0x58, 0x0C), (0xFB, 0xBF, 0x24), (0xFE, 0xF3, 0xC7), (0x1C, 0x19, 0x17)])
def solnedgang(x, y, w, h):
    if y > h * 0.72:
        return 4
    d = math.hypot(x - w * 0.6, y - h * 0.72)
    if d < 36:
        return 3
    return min(2, int(y / (h * 0.24)))


def main():
    for name, w, h, palette, fn in IMAGES:
        im = Image.new("P", (w, h))
        flat = [c for rgb in palette for c in rgb]
        im.putpalette(flat + [0] * (768 - len(flat)))
        im.putdata([fn(x, y, w, h) for y in range(h) for x in range(w)])
        path = os.path.join(HERE, name)
        im.save(path, optimize=True)
        size = os.path.getsize(path)
        assert size < 20_000, f"{name}: {size} bytes"
        print(f"{name}: {w}x{h}, {size} bytes")


if __name__ == "__main__":
    main()

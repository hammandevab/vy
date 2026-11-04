#!/usr/bin/env python3
"""Procedural product images for modules/shop: eight 240 x 240 RGB PNGs, each
well under 20 KB, drawn with Pillow from fixed numbers so the files are
reproducible. Run from anywhere: python3 modules/shop/assets/generate.py"""
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
SIZE = 240

# (file, background, ink, motif)
PRODUCTS = [
    ("01-mugg", (0xE8, 0xD5, 0xB7), (0x6B, 0x3F, 0x2A), "mug"),
    ("02-bok", (0xD6, 0xE4, 0xF0), (0x1F, 0x3A, 0x5F), "book"),
    ("03-lampa", (0xFF, 0xF1, 0xC9), (0xB4, 0x7A, 0x1E), "lamp"),
    ("04-vaxt", (0xE3, 0xF1, 0xDD), (0x2F, 0x6B, 0x3A), "plant"),
    ("05-horlurar", (0xE6, 0xE1, 0xF3), (0x3F, 0x2E, 0x6B), "headphones"),
    ("06-ryggsack", (0xF3, 0xDF, 0xD6), (0x8A, 0x3B, 0x2E), "backpack"),
    ("07-termos", (0xDA, 0xEC, 0xEC), (0x1E, 0x5F, 0x66), "thermos"),
    ("08-plad", (0xF1, 0xE4, 0xE9), (0x7A, 0x2E, 0x4C), "blanket"),
]


def mix(a, b, t):
    return tuple(int(round(x + (y - x) * t)) for x, y in zip(a, b))


def draw(motif, d, bg, ink):
    light = mix(ink, (255, 255, 255), 0.35)
    if motif == "mug":
        d.rounded_rectangle((70, 80, 160, 185), 14, fill=ink)
        d.ellipse((150, 100, 200, 160), outline=ink, width=12)
        d.rounded_rectangle((82, 92, 148, 110), 6, fill=light)
    elif motif == "book":
        d.rounded_rectangle((60, 50, 180, 195), 8, fill=ink)
        d.rectangle((60, 50, 78, 195), fill=light)
        for y in range(80, 170, 18):
            d.line((95, y, 165, y), fill=light, width=4)
    elif motif == "lamp":
        d.polygon([(120, 50), (60, 130), (180, 130)], fill=ink)
        d.rectangle((114, 130, 126, 180), fill=light)
        d.ellipse((80, 170, 160, 200), fill=ink)
    elif motif == "plant":
        d.polygon([(90, 160), (150, 160), (140, 200), (100, 200)], fill=ink)
        for cx, cy, r in [(120, 110, 32), (85, 130, 26), (155, 130, 26), (120, 70, 24)]:
            d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=light)
        d.line((120, 160, 120, 95), fill=ink, width=6)
    elif motif == "headphones":
        d.arc((50, 50, 190, 190), 180, 360, fill=ink, width=14)
        d.rounded_rectangle((45, 115, 85, 175), 12, fill=ink)
        d.rounded_rectangle((155, 115, 195, 175), 12, fill=ink)
        d.rounded_rectangle((55, 125, 75, 165), 8, fill=light)
        d.rounded_rectangle((165, 125, 185, 165), 8, fill=light)
    elif motif == "backpack":
        d.rounded_rectangle((65, 60, 175, 200), 24, fill=ink)
        d.arc((90, 30, 150, 90), 180, 360, fill=ink, width=12)
        d.rounded_rectangle((85, 130, 155, 180), 12, fill=light)
        d.rectangle((110, 60, 130, 120), fill=light)
    elif motif == "thermos":
        d.rounded_rectangle((85, 60, 155, 205), 18, fill=ink)
        d.rounded_rectangle((95, 35, 145, 70), 10, fill=light)
        d.rectangle((85, 120, 155, 160), fill=light)
    elif motif == "blanket":
        d.rounded_rectangle((40, 60, 200, 180), 16, fill=ink)
        for i, y in enumerate(range(70, 175, 16)):
            d.rectangle((48, y, 192, y + 7), fill=light if i % 2 == 0 else bg)
        d.rectangle((40, 170, 200, 180), fill=light)


def main():
    for name, bg, ink, motif in PRODUCTS:
        img = Image.new("RGB", (SIZE, SIZE), bg)
        d = ImageDraw.Draw(img)
        # A soft diagonal band so the image is not a flat fill.
        band = mix(bg, (255, 255, 255), 0.5)
        d.polygon([(0, SIZE), (SIZE, 0), (SIZE, 70), (70, SIZE)], fill=band)
        draw(motif, d, bg, ink)
        path = HERE / f"{name}.png"
        img.save(path, optimize=True)
        print(f"{path.name}: {path.stat().st_size} bytes")


if __name__ == "__main__":
    main()

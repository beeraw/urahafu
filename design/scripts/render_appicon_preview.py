#!/usr/bin/env python3
"""Build the app icon preview: sizes 512/256/128/64/32/16 rendered from the SVG on light and dark strips.

Reads app-icon.svg next to this script's design/app-icon/ dir and writes preview.png there.
"""
import os
import subprocess
import tempfile
from PIL import Image

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
BASE = os.path.join(SCRIPT_DIR, "..", "app-icon")
SVG = os.path.join(BASE, "app-icon.svg")
OUT = os.path.join(BASE, "preview.png")
SIZES = [512, 256, 128, 64, 32, 16]

pad = 24
gap = 20
row_h = 512 + pad * 2
light_bg = (0xF5, 0xF5, 0xF7)
dark_bg = (0x1E, 0x1E, 0x1E)
total_w = pad + sum(SIZES) + gap * (len(SIZES) - 1) + pad

with tempfile.TemporaryDirectory() as tmp:
    renders = {}
    for s in SIZES:
        path = os.path.join(tmp, f"preview_{s}.png")
        subprocess.run(["rsvg-convert", "-w", str(s), "-h", str(s), SVG, "-o", path], check=True)
        renders[s] = Image.open(path).convert("RGBA")

    def make_row(bg):
        img = Image.new("RGBA", (total_w, row_h), bg + (255,))
        x = pad
        for s in SIZES:
            y = pad + (512 - s)
            img.alpha_composite(renders[s], (x, y))
            x += s + gap
        return img

    row_light = make_row(light_bg)
    row_dark = make_row(dark_bg)

row_gap = 16
out = Image.new("RGBA", (total_w, row_h * 2 + row_gap), (30, 30, 30, 255))
out.alpha_composite(row_light, (0, 0))
out.alpha_composite(row_dark, (0, row_h + row_gap))
out.convert("RGB").save(OUT)

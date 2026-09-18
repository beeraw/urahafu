#!/usr/bin/env python3
"""Build the menu-bar icon preview strip: light/dark bars with 1x/2x icons plus a x4 nearest-neighbour zoom.

Reads menubar.png / menubar@2x.png next to this script's design/menubar-icon/ dir and writes preview.png there.
"""
import os
from PIL import Image

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
BASE = os.path.join(SCRIPT_DIR, "..", "menubar-icon")

icon1x = Image.open(os.path.join(BASE, "menubar.png")).convert("RGBA")
icon2x = Image.open(os.path.join(BASE, "menubar@2x.png")).convert("RGBA")


def tint(img, rgb):
    """Recolor all opaque pixels to `rgb`, keeping the alpha channel."""
    r, g, b = rgb
    out = Image.new("RGBA", img.size)
    px_in = img.load()
    px_out = out.load()
    for y in range(img.size[1]):
        for x in range(img.size[0]):
            _, _, _, pa = px_in[x, y]
            px_out[x, y] = (r, g, b, pa)
    return out


strip_h = 44
pad = 14
light_bg = (0xE8, 0xE8, 0xE8)
dark_bg = (0x2B, 0x2B, 0x2B)
zoom = 4

icon1x_light = tint(icon1x, (0, 0, 0))
icon2x_light = tint(icon2x, (0, 0, 0))
icon1x_dark = tint(icon1x, (255, 255, 255))
icon2x_dark = tint(icon2x, (255, 255, 255))

zoomed_light = icon1x_light.resize((18 * zoom, 18 * zoom), Image.NEAREST)
zoomed_dark = icon1x_dark.resize((18 * zoom, 18 * zoom), Image.NEAREST)


def make_strip(bg, icon1, icon2, zoomed):
    w = pad + 18 + pad * 2 + 36 + pad * 2 + 18 * zoom + pad
    img = Image.new("RGBA", (w, strip_h), bg + (255,))
    y1 = (strip_h - 18) // 2
    img.alpha_composite(icon1, (pad, y1))
    y2 = (strip_h - 36) // 2
    img.alpha_composite(icon2, (pad + 18 + pad * 2, y2))
    yz = (strip_h - 18 * zoom) // 2
    img.alpha_composite(zoomed, (pad + 18 + pad * 2 + 36 + pad * 2, yz))
    return img


strip_light = make_strip(light_bg, icon1x_light, icon2x_light, zoomed_light)
strip_dark = make_strip(dark_bg, icon1x_dark, icon2x_dark, zoomed_dark)

gap = 10
total_w = max(strip_light.width, strip_dark.width)
total_h = strip_light.height + gap + strip_dark.height
out = Image.new("RGBA", (total_w, total_h), (30, 30, 30, 255))
out.alpha_composite(strip_light, (0, 0))
out.alpha_composite(strip_dark, (0, strip_light.height + gap))
out.convert("RGB").save(os.path.join(BASE, "preview.png"))

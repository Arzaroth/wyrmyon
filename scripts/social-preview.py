#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow==12.3.0"]
# ///
# Draws assets/social.png, the 1280x640 picture GitHub shows when the
# repository is shared (Settings, Social preview), from assets/logo.png's
# source: the cut-out emblem, the name and the tagline. It uses Ubuntu Sans
# and JetBrains Mono from their Nerd Font builds, in FONTS or
# /usr/share/fonts/nerd-fonts.
#
#   scripts/social-preview.py
import importlib.util
import os
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H = 1280, 640
ROOT = Path(__file__).resolve().parent.parent
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("qr_logo", ROOT / "scripts/qr-logo.py")
qr_logo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qr_logo)
FONTS = Path(os.environ.get("FONTS", "/usr/share/fonts/nerd-fonts"))
emblem = qr_logo.emblem(ROOT / "assets/emblem.jpg")
out = ROOT / "assets/social.png"

bg = Image.new("RGB", (W, H))
px = bg.load()
for y in range(H):
    for x in range(W):
        t = (x / W) * 0.6 + (y / H) * 0.4
        px[x, y] = (int(7 + 6 * t), int(10 + 10 * t), int(28 + 30 * t))
bg = bg.convert("RGBA")

size = 520
e = emblem.copy()
e.thumbnail((size, size), Image.LANCZOS)
cx, cy = 330, H // 2
glow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
d = ImageDraw.Draw(glow)
for r, a in ((330, 40), (290, 55), (250, 70)):
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=(40, 170, 255, a))
glow = glow.filter(ImageFilter.GaussianBlur(60))
bg.alpha_composite(glow)
bg.alpha_composite(e, (cx - e.width // 2, cy - e.height // 2))

draw = ImageDraw.Draw(bg)
fonts = FONTS / "UbuntuSans"
title = ImageFont.truetype(fonts / "UbuntuSansNerdFont-Bold.ttf", 116)
tag = ImageFont.truetype(fonts / "UbuntuSansNerdFont-Regular.ttf", 40)
left = 660
top = 188
box = draw.textbbox((left, top), "wyrmyon", font=title)
draw.text((left, top), "wyrmyon", font=title, fill=(232, 244, 255))
rule = box[3] + 22
draw.line((left + 6, rule, left + 260, rule), fill=(60, 120, 230), width=4)
draw.text((left + 4, rule + 24), "The magic-wormhole,", font=tag, fill=(130, 210, 255))
draw.text((left + 4, rule + 74), "with a wyrm in it.", font=tag, fill=(130, 210, 255))
mono = ImageFont.truetype(FONTS / "JetBrainsMono/JetBrainsMonoNerdFont-Regular.ttf", 24)
draw.text((left + 6, rule + 160), "powered by tachyons", font=mono, fill=(82, 120, 190))
bg.convert("RGB").save(out, optimize=True)

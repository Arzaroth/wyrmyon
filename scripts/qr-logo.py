#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow==12.3.0"]
# ///
# Builds the project logo and the wyrm drawn in the middle of `wyrm send --qr`
# from the round emblem on a white background (assets/emblem.png unless
# another PICTURE is given): assets/logo.png, the README's logo;
# crates/cli/assets/qr-wyrm.png, the picture a terminal with the kitty
# graphics protocol shows; and crates/cli/src/qr/logo.rs, the same emblem as
# 256-colour pixels for the other terminals, 0 where the QR code's own
# modules show through. The white outside the emblem's circle becomes
# transparent. uv fetches the pinned Pillow, so the output is reproducible.
#
#   scripts/qr-logo.py [PICTURE]
import sys
from pathlib import Path

from PIL import Image

COLS, ROWS = 13, 7
PX = 20
ROOT = Path(__file__).resolve().parent.parent


def white(p):
    return min(p[:3]) > 225


def emblem(picture):
    image = Image.open(picture).convert("RGBA")
    pixels = image.load()
    x0, y0, x1, y1 = image.convert("L").point(lambda v: 255 if v < 226 else 0).getbbox()
    r = (x1 - x0) / 2
    cx, cy = (x0 + x1) / 2, y1 - r
    for y in range(image.height):
        for x in range(image.width):
            if white(pixels[x, y]) and (x - cx) ** 2 + (y - cy) ** 2 > r * r:
                pixels[x, y] = (0, 0, 0, 0)
    return image.crop(image.getbbox())


def fit(image):
    width, height = COLS * PX, ROWS * 2 * PX
    room = ((COLS - 2) * PX, (ROWS * 2 - 2) * PX)
    scale = min(room[0] / image.width, room[1] / image.height)
    image = image.resize((round(image.width * scale), round(image.height * scale)), Image.LANCZOS)
    canvas = Image.new("RGBA", (width, height))
    canvas.paste(image, ((width - image.width) // 2, (height - image.height) // 2))
    return canvas


def cube(v):
    return min(range(6), key=lambda i: abs([0, 95, 135, 175, 215, 255][i] - v))


def pixels(canvas):
    small = canvas.resize((COLS, ROWS * 2), Image.BOX).load()
    rows = []
    for y in range(ROWS * 2):
        row = []
        for x in range(COLS):
            r, g, b, a = small[x, y]
            row.append(16 + 36 * cube(r) + 6 * cube(g) + cube(b) if a > 128 else 0)
        rows.append(row)
    return rows


def main():
    image = emblem(sys.argv[1] if len(sys.argv) > 1 else ROOT / "assets/emblem.png")
    logo = image.copy()
    logo.thumbnail((256, 256), Image.LANCZOS)
    logo.save(ROOT / "assets/logo.png", optimize=True)
    canvas = fit(image)
    canvas.save(ROOT / "crates/cli/assets/qr-wyrm.png", optimize=True)
    rows = ",\n".join(f"    {row}" for row in pixels(canvas))
    text = (
        'pub const PNG: &[u8] = include_bytes!("../../assets/qr-wyrm.png");\n'
        f"pub const COLS: usize = {COLS};\n"
        f"pub const ROWS: usize = {ROWS};\n"
        "#[rustfmt::skip]\n"
        f"pub const PIXELS: [[u8; COLS]; ROWS * 2] = [\n{rows},\n];\n"
    )
    (ROOT / "crates/cli/src/qr/logo.rs").write_text(text)


main()

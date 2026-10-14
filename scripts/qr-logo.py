#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow==12.3.0"]
# ///
# Builds every picture of the wyrm emblem from the emblem on a white
# background (assets/emblem.jpg unless another PICTURE is given):
# assets/logo.png, the README's logo; assets/wyrmyon.ico, the Windows icon of
# the binaries and the MSI; crates/cli/assets/qr-wyrm.png, the picture a
# terminal with the kitty graphics protocol shows in `wyrm send --qr`; and
# crates/cli/src/qr/logo.rs, the same emblem as 256-colour pixels for the
# other terminals, 0 where the QR code's own modules show through. Only the
# white connected to the picture's edges becomes transparent, and the white
# is taken out of the colours along that edge so no pale halo is left. uv
# fetches the pinned Pillow, so the output is reproducible.
#
#   scripts/qr-logo.py [PICTURE]
import sys
from pathlib import Path

from collections import deque

from PIL import Image, ImageFilter

COLS, ROWS = 13, 7
PX = 20
ROOT = Path(__file__).resolve().parent.parent


def emblem(picture):
    image = Image.open(picture).convert("RGB")
    width, height = image.size
    source = image.load()
    outside = bytearray(width * height)
    queue = deque((x, y) for x in range(width) for y in (0, height - 1))
    queue.extend((x, y) for y in range(height) for x in (0, width - 1))
    while queue:
        x, y = queue.popleft()
        if outside[y * width + x] or min(source[x, y]) < 200:
            continue
        outside[y * width + x] = 1
        for nx, ny in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
            if 0 <= nx < width and 0 <= ny < height:
                queue.append((nx, ny))
    mask = Image.frombytes("L", (width, height), bytes(255 * v for v in outside))
    edge = mask.filter(ImageFilter.MaxFilter(5)).load()
    cut = Image.new("RGBA", (width, height))
    out = cut.load()
    for y in range(height):
        for x in range(width):
            pixel = source[x, y]
            if not edge[x, y]:
                out[x, y] = pixel + (255,)
                continue
            alpha = min(1.0, max(255 - c for c in pixel) / 255 * 1.15)
            if alpha < 0.08 * 1.15 and outside[y * width + x]:
                continue
            if alpha > 0:
                lift = [round((c - 255 * (1 - alpha)) / alpha) for c in pixel]
                out[x, y] = tuple(max(0, min(255, c)) for c in lift) + (round(alpha * 255),)
    return cut.crop(cut.getbbox())


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
    image = emblem(sys.argv[1] if len(sys.argv) > 1 else ROOT / "assets/emblem.jpg")
    logo = image.copy()
    logo.thumbnail((256, 256), Image.LANCZOS)
    logo.save(ROOT / "assets/logo.png", optimize=True)
    side = max(image.size)
    square = Image.new("RGBA", (side, side))
    square.paste(image, ((side - image.width) // 2, (side - image.height) // 2))
    sizes = [(n, n) for n in (16, 24, 32, 48, 64, 128, 256)]
    square.save(ROOT / "assets/wyrmyon.ico", sizes=sizes)
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

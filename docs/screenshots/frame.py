"""Presentation frame for README images: a soft gradient backdrop, padding and a drop shadow.

The window itself (rounded corners, title bar with the three dots) comes from the capture:
``term.py`` renders through Rich's SVG export, whose transparent corners this frame keeps.
A plain screenshot without its own chrome gets one added (``chrome=True``).

    uv run --with pillow python docs/screenshots/frame.py in.png out.png [--chrome] [--title T]
"""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

TOP, BOTTOM = (31, 37, 71), (66, 45, 99)  # slate indigo -> muted violet, diagonal
PAD = 80
SHADOW_BLUR, SHADOW_OFFSET, SHADOW_ALPHA = 28, 18, 150


def gradient(size: tuple[int, int]) -> Image.Image:
    w, h = size
    small = Image.new("RGB", (64, 64))
    px = small.load()
    for y in range(64):
        for x in range(64):
            t = (x / 63 * 0.45 + y / 63 * 0.55)
            px[x, y] = tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM))
    return small.resize((w, h), Image.BICUBIC)


def add_chrome(img: Image.Image, title: str = "") -> Image.Image:
    """Wrap a bare screenshot in a macOS-style window: title bar, three dots, rounded corners."""
    bar, radius = 44, 14
    w, h = img.size
    win = Image.new("RGBA", (w, h + bar), (0, 0, 0, 0))
    body = Image.new("RGBA", win.size, (40, 42, 54, 255))
    body.paste(img.convert("RGBA"), (0, bar))
    draw = ImageDraw.Draw(body)
    for i, color in enumerate(((255, 95, 87), (254, 188, 46), (40, 200, 64))):
        cx, cy = 26 + i * 24, bar // 2
        draw.ellipse((cx - 7, cy - 7, cx + 7, cy + 7), fill=color)
    if title:
        try:
            font = ImageFont.truetype("DejaVuSans-Bold.ttf", 18)
        except OSError:
            font = ImageFont.load_default()
        tw = draw.textlength(title, font=font)
        draw.text(((w - tw) / 2, bar / 2 - 11), title, fill=(200, 202, 214), font=font)
    mask = Image.new("L", win.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, w - 1, h + bar - 1), radius, fill=255)
    win.paste(body, (0, 0), mask)
    return win


def frame(img: Image.Image, pad: int = PAD) -> Image.Image:
    """``img`` (RGBA, transparent outside the window) on the gradient, with a soft shadow."""
    img = img.convert("RGBA")
    w, h = img.size
    canvas = gradient((w + 2 * pad, h + 2 * pad)).convert("RGBA")
    alpha = img.getchannel("A")
    shadow = Image.new("L", canvas.size, 0)
    shadow.paste(alpha.point(lambda a: SHADOW_ALPHA if a else 0), (pad, pad + SHADOW_OFFSET))
    shadow = shadow.filter(ImageFilter.GaussianBlur(SHADOW_BLUR))
    canvas.paste(Image.new("RGBA", canvas.size, (8, 8, 20, 255)), (0, 0), shadow)
    canvas.alpha_composite(img, (pad, pad))
    return canvas.convert("RGB")


def frame_file(src: Path, dst: Path, chrome: bool = False, title: str = "", pad: int = PAD) -> None:
    img = Image.open(src)
    if chrome:
        img = add_chrome(img, title)
    frame(img, pad).save(dst, optimize=True)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("src", type=Path)
    ap.add_argument("dst", type=Path)
    ap.add_argument("--chrome", action="store_true", help="add a window title bar (bare screenshots)")
    ap.add_argument("--title", default="")
    ap.add_argument("--pad", type=int, default=PAD)
    args = ap.parse_args()
    frame_file(args.src, args.dst, args.chrome, args.title, args.pad)


if __name__ == "__main__":
    main()

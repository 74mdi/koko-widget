#!/usr/bin/env python3
"""Generate the KOKO WIDGET application icon at every hicolor size.

This script is intentionally dependency-light (Pillow only) and is not part of
the runtime. Run it after changing the icon design:

    python3 assets/icons/generate_icon.py
"""

from __future__ import annotations

import math
import os
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent
HICOLOR = ROOT / "hicolor"
S = 1536  # supersampled working canvas
OUTPUT = 512


def lerp(a: float, b: float, t: float) -> float:
    return a + (b - a) * t


def rounded_mask(size: int, radius: int) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, size - 1, size - 1), radius=radius, fill=255
    )
    return mask


def gradient(size: int, top: tuple[int, int, int], bottom: tuple[int, int, int]) -> Image.Image:
    image = Image.new("RGBA", (size, size))
    pixels = image.load()
    for y in range(size):
        t = y / (size - 1)
        # Add a gentle diagonal shift so the icon has depth.
        for x in range(size):
            diagonal = (x / (size - 1)) * 0.12
            c = tuple(
                round(lerp(top[i], bottom[i], min(1.0, t + diagonal)))
                for i in range(3)
            )
            pixels[x, y] = (*c, 255)
    return image


def build_icon() -> Image.Image:
    icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # Rounded squircle.
    margin = 54
    size = S - margin * 2
    base = gradient(size, (38, 30, 60), (9, 8, 15))
    mask = rounded_mask(size, 306)

    # Purple atmospheric glow.
    glow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    gp = glow.load()
    cx, cy = size * 0.30, size * 0.22
    max_d = size * 0.72
    for y in range(size):
        for x in range(size):
            d = math.hypot(x - cx, y - cy) / max_d
            if d < 1.0:
                alpha = int((1.0 - d) ** 2 * 120)
                gp[x, y] = (113, 92, 255, alpha)
    glow = glow.filter(ImageFilter.GaussianBlur(80))
    base = Image.alpha_composite(base, glow)

    # Cool rim from the upper right.
    rim_glow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    rg = rim_glow.load()
    for y in range(size):
        for x in range(size):
            d = math.hypot(x - size * 0.88, y - size * 0.08) / (size * 0.55)
            if d < 1.0:
                rg[x, y] = (128, 92, 200, int((1.0 - d) ** 3 * 74))
    base = Image.alpha_composite(base, rim_glow.filter(ImageFilter.GaussianBlur(55)))

    icon.paste(base, (margin, margin), mask)

    # Outer border and very soft inner bevel.
    border = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    bd = ImageDraw.Draw(border)
    bd.rounded_rectangle(
        (margin, margin, S - margin - 1, S - margin - 1),
        radius=306,
        outline=(255, 255, 255, 38),
        width=5,
    )
    bd.arc(
        (margin + 5, margin + 5, S - margin - 5, S - margin - 5),
        start=190,
        end=320,
        fill=(255, 255, 255, 70),
        width=3,
    )
    icon = Image.alpha_composite(icon, border)

    # Floating glass pill.
    pill = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    pd = ImageDraw.Draw(pill)
    px0, py0, px1, py1 = 170, 548, S - 170, 974
    radius = 190
    pd.rounded_rectangle(
        (px0, py0 + 16, px1, py1 + 22),
        radius=radius,
        fill=(0, 0, 0, 85),
    )
    pill = pill.filter(ImageFilter.GaussianBlur(38))

    glass = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glass)
    gd.rounded_rectangle(
        (px0, py0, px1, py1),
        radius=radius,
        fill=(249, 246, 255, 48),
        outline=(255, 255, 255, 116),
        width=5,
    )
    # Glass top sheen.
    sheen = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    sd = ImageDraw.Draw(sheen)
    sd.rounded_rectangle(
        (px0 + 5, py0 + 5, px1 - 5, py0 + 190),
        radius=radius - 4,
        fill=(255, 255, 255, 26),
    )
    sheen = sheen.filter(ImageFilter.GaussianBlur(18))
    glass = Image.alpha_composite(glass, sheen)

    icon = Image.alpha_composite(icon, pill)
    icon = Image.alpha_composite(icon, glass)

    # Mini album art with a hint of a cover.
    art_size = 286
    ax0, ay0 = px0 + 50, py0 + 50
    art = gradient(art_size, (139, 124, 246), (55, 174, 174))
    art_mask = rounded_mask(art_size, 70)
    # A few oversized graphic strokes give it the feeling of a cover.
    ad = ImageDraw.Draw(art)
    ad.ellipse((28, 104, 262, 338), fill=(255, 255, 255, 38))
    ad.ellipse((98, 36, 304, 242), fill=(18, 12, 32, 48))
    ad.arc((28, 88, 280, 340), start=200, end=340, fill=(255, 255, 255, 80), width=8)
    icon.paste(art, (ax0, ay0), art_mask)
    rim = ImageDraw.Draw(icon)
    rim.rounded_rectangle(
        (ax0, ay0, ax0 + art_size, ay0 + art_size),
        radius=70,
        outline=(255, 255, 255, 64),
        width=3,
    )

    # Title and artist placeholder bars.
    title_x = 530
    y0 = 676
    bar_color = (255, 255, 255, 220)
    rim.rounded_rectangle((title_x, y0, title_x + 400, y0 + 42), radius=21, fill=bar_color)
    rim.rounded_rectangle(
        (title_x, y0 + 76, title_x + 280, y0 + 106),
        radius=15,
        fill=(255, 255, 255, 132),
    )

    # Play transport glyph.
    pcx, pcy = 1192, 760
    triangle = [
        (pcx - 58, pcy - 84),
        (pcx + 88, pcy),
        (pcx - 58, pcy + 84),
    ]
    rim.polygon(triangle, fill=(255, 255, 255, 248))
    # A small next marker.
    rim.rounded_rectangle(
        (1310, pcy - 76, 1332, pcy + 76), radius=11, fill=(255, 255, 255, 205)
    )

    # Gentle bottom reflection.
    reflection = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    rd = ImageDraw.Draw(reflection)
    rd.ellipse((300, 1030, S - 300, 1230), fill=(130, 110, 255, 34))
    reflection = reflection.filter(ImageFilter.GaussianBlur(70))
    icon = Image.alpha_composite(icon, reflection)

    return icon.resize((OUTPUT, OUTPUT), Image.Resampling.LANCZOS)


def main() -> None:
    source = build_icon()
    (ROOT / "io.github.kokowidget.KokoWidget.png").parent.mkdir(parents=True, exist_ok=True)
    source.save(ROOT / "io.github.kokowidget.KokoWidget.png")
    for size in (16, 24, 32, 48, 64, 128, 256, 512):
        directory = HICOLOR / f"{size}x{size}" / "apps"
        directory.mkdir(parents=True, exist_ok=True)
        image = source.resize((size, size), Image.Resampling.LANCZOS)
        image.save(directory / "io.github.kokowidget.KokoWidget.png", optimize=True)
    os.chmod(ROOT / "io.github.kokowidget.KokoWidget.png", 0o644)


if __name__ == "__main__":
    main()

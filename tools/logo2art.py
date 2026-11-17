#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
"""Regenerate the Steelbore OS banner art in assets/.

Usage: tools/logo2art.py [path/to/steelbore-os.svg]

The emblem (Bravais's `assets/brand/steelbore-os.svg`, which has no text) is
trimmed, scaled to 36 columns and packed into half-blocks (U+2580, U+2584,
U+2588). The Linux console draws those from its own font, and Gantry adds any
the font lacks (see `src/vtfont.rs`). The wordmark is composed from a small
3-row font. Needs Pillow, plus `rsvg-convert` or CairoSVG to rasterise.
"""
import io
import pathlib
import shutil
import subprocess
import sys

from PIL import Image, ImageOps

COLS = 36
THRESHOLD = 150  # 0..255; darker than this is ink
DEFAULT_SVG = "/spacecraft-software/bravais/assets/brand/steelbore-os.svg"
ASSETS = pathlib.Path(__file__).resolve().parent.parent / "assets"

FONT = {
    "S": ("▄▀▀▀", " ▀▀▄", "▀▀▀ "),
    "T": ("▀▀█▀▀", "  █  ", "  ▀  "),
    "E": ("█▀▀▀", "█▀▀ ", "▀▀▀▀"),
    "L": ("█   ", "█   ", "▀▀▀▀"),
    "B": ("█▀▀▄", "█▀▀▄", "▀▀▀ "),
    "O": ("▄▀▀▄", "█  █", " ▀▀ "),
    "R": ("█▀▀▄", "█▀█ ", "▀ ▀▄"),
    " ": (" ", " ", " "),
}


def rasterise(svg: str) -> Image.Image:
    if shutil.which("rsvg-convert"):
        png = subprocess.run(
            ["rsvg-convert", "-w", "1200", "-b", "white", svg],
            check=True,
            capture_output=True,
        ).stdout
    else:
        import cairosvg

        png = cairosvg.svg2png(
            url=svg, output_width=1200, background_color="white"
        )
    return Image.open(io.BytesIO(png)).convert("L")


def logo(svg: str) -> str:
    im = rasterise(svg)
    box = ImageOps.invert(im).point(lambda v: 255 if v > 40 else 0).getbbox()
    im = im.crop(box)
    h = round(im.height * COLS / im.width)
    h += h % 2
    px = im.resize((COLS, h), Image.BOX).load()
    glyph = {(0, 0): " ", (1, 0): "▀", (0, 1): "▄", (1, 1): "█"}
    rows = []
    for y in range(0, h, 2):
        rows.append(
            "".join(
                glyph[(int(px[x, y] < THRESHOLD), int(px[x, y + 1] < THRESHOLD))]
                for x in range(COLS)
            ).rstrip()
        )
    return "\n".join(rows) + "\n"


def wordmark(text: str) -> str:
    rows = ["", "", ""]
    for ch in text:
        for i, part in enumerate(FONT[ch]):
            rows[i] += part + " "
    return "\n".join(r.rstrip() for r in rows) + "\n"


if __name__ == "__main__":
    svg = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_SVG
    (ASSETS / "steelbore-logo.txt").write_text(logo(svg), encoding="utf-8")
    (ASSETS / "steelbore-wordmark.txt").write_text(wordmark("STEELBORE OS"), encoding="utf-8")

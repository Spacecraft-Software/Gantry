#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
"""Regenerate the Steelbore OS banner art in assets/.

Usage: tools/logo2art.py [path/to/steelbore_logo.svg]

Needs rsvg-convert and ImageMagick (magick). The logo is trimmed, scaled to
56 columns and packed into half-blocks (U+2580, U+2584, U+2588), which the
Linux console font carries. The wordmark is composed from a small 3-row font.
"""
import pathlib
import subprocess
import sys
import tempfile

COLS = 56
THRESHOLD = 160  # 0..255; darker than this is ink
DEFAULT_SVG = "/spacecraft-software/branding/logo/steelbore/steelbore_logo.svg"
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


def logo(svg: str) -> str:
    with tempfile.TemporaryDirectory() as tmp:
        png = pathlib.Path(tmp) / "logo.png"
        subprocess.run(
            ["rsvg-convert", "-w", "1200", "-b", "white", svg, "-o", str(png)],
            check=True,
        )
        pgm = subprocess.run(
            [
                "magick", str(png), "-trim", "+repage", "-colorspace", "Gray",
                "-filter", "Box", "-resize", f"{COLS}x", "-depth", "8", "pgm:-",
            ],
            check=True,
            capture_output=True,
        ).stdout
    _, size, _, data = pgm.split(b"\n", 3)
    w, h = map(int, size.split())
    ink = lambda x, y: y < h and data[y * w + x] < THRESHOLD
    glyph = {(0, 0): " ", (1, 0): "▀", (0, 1): "▄", (1, 1): "█"}
    rows = []
    for y in range(0, h, 2):
        rows.append(
            "".join(glyph[(int(ink(x, y)), int(ink(x, y + 1)))] for x in range(w)).rstrip()
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

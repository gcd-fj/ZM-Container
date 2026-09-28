#!/usr/bin/env python3
"""Convert the existing application icon for native bundles; requires Pillow."""
from pathlib import Path

from PIL import Image

assets = Path(__file__).resolve().parents[1] / "assets"
with Image.open(assets / "io.github.gcd-fj.zm-linux.png") as source:
    icon = source.convert("RGBA")
    icon.save(assets / "zm.ico", sizes=[(s, s) for s in (16, 32, 48, 64, 128, 256)])
    icon.save(assets / "zm.icns")

#!/usr/bin/env python3
"""Generate app icons from a source PNG.

Takes a source PNG (argv[1]) and regenerates the full icon set at standard
sizes for Linux hicolor, macOS, and Windows platforms.

Usage:
    python3 assets/gen-icons.py <source.png>

Produces:
    - icon.png: source copied verbatim if square, padded to square if not (transparent margin).
    - icon-32.png, icon-64.png, icon-128.png, icon-256.png, icon-512.png: LANCZOS resizes.
    - icon.ico: multi-size ICO container with 16/32/48/64/128/256 entries.
    - icon.icns: Apple ICNS (skipped with warning if Pillow can't generate it on this system).

Requires Pillow (PIL).
"""
import os
import sys
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
SIZES = [32, 64, 128, 256, 512]
ICO_SIZES = [16, 32, 48, 64, 128, 256]


def pad_to_square(img):
    """Pad image to square with transparent margin if not already square."""
    w, h = img.size
    if w == h:
        return img

    size = max(w, h)
    padded = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    x = (size - w) // 2
    y = (size - h) // 2
    padded.paste(img, (x, y), img if img.mode == "RGBA" else None)
    return padded


def main():
    if len(sys.argv) < 2:
        print("Usage: python3 gen-icons.py <source.png>")
        sys.exit(1)

    source_path = sys.argv[1]

    if not os.path.exists(source_path):
        print(f"Error: source PNG not found: {source_path}")
        sys.exit(1)

    # Load and convert to RGBA
    master = Image.open(source_path).convert("RGBA")

    # Ensure square
    master = pad_to_square(master)

    # Save master as icon.png
    icon_path = os.path.join(HERE, "icon.png")
    master.save(icon_path, format="PNG")
    print(f"wrote icon.png ({master.width}x{master.height})")

    # Generate PNG derivatives at standard sizes
    for s in SIZES:
        resized = master.resize((s, s), Image.LANCZOS)
        name = f"icon-{s}.png"
        resized.save(os.path.join(HERE, name), format="PNG")
        print(f"wrote {name} ({s}x{s})")

    # Generate ICO with multiple sizes
    ico_images = []
    for s in ICO_SIZES:
        if s == master.width:
            ico_images.append(master)
        else:
            ico_images.append(master.resize((s, s), Image.LANCZOS))

    ico_path = os.path.join(HERE, "icon.ico")
    master.save(
        ico_path,
        format="ICO",
        sizes=[(s, s) for s in ICO_SIZES],
    )
    print(f"wrote icon.ico ({', '.join(str(s) for s in ICO_SIZES)})")

    # Try to generate ICNS (Apple format); skip with warning if not supported
    icns_path = os.path.join(HERE, "icon.icns")
    try:
        master.save(icns_path, format="ICNS")
        print(f"wrote icon.icns ({master.width}x{master.height})")
    except Exception as e:
        print(f"warning: could not write icon.icns (Pillow ICNS support not available on this system): {e}")
        # Remove the file if it was partially written
        if os.path.exists(icns_path):
            os.remove(icns_path)


if __name__ == "__main__":
    main()

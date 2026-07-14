#!/usr/bin/env python3
"""Generate placeholder app icons for koma.

Produces a solid dark-navy (#1a1a2e) rounded square with a simple light "K"
glyph, plus every size cargo-packager's deb/hicolor path and Windows .ico
format want. This is throwaway placeholder art — replace assets/icon.png
(and re-run this script) with final branding when it's ready.

Usage:
    python3 assets/gen-placeholder-icons.py

Requires Pillow (PIL). If Pillow is not installed, falls back to hand-rolled
solid-color PNG/ICO generation (no rounded corners, no glyph) via zlib/struct
so the pipeline never has a hard PIL dependency.
"""
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
BG_COLOR = (0x1A, 0x1A, 0x2E, 0xFF)  # #1a1a2e, fully opaque
GLYPH_COLOR = (0xEA, 0xEA, 0xF5, 0xFF)  # light near-white glyph

SIZES = [32, 64, 128, 256, 512]
ICO_SIZES = [16, 32, 48, 64, 128, 256]

try:
    from PIL import Image, ImageDraw

    HAVE_PIL = True
except ImportError:
    HAVE_PIL = False


def build_master_pil(size=1024):
    """Rounded dark-navy square with a simple light 'K' glyph."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    margin = int(size * 0.04)
    radius = int(size * 0.18)
    draw.rounded_rectangle(
        [margin, margin, size - margin, size - margin],
        radius=radius,
        fill=BG_COLOR,
    )

    # Hand-drawn "K" glyph using thick polygon strokes so we don't depend on
    # any particular font being installed.
    stroke = int(size * 0.09)
    cx = size * 0.5
    top = size * 0.28
    bottom = size * 0.72
    left = size * 0.34

    # Vertical bar of the K.
    draw.rectangle([left, top, left + stroke, bottom], fill=GLYPH_COLOR)

    # Upper diagonal stroke.
    draw.polygon(
        [
            (left + stroke, top + (bottom - top) * 0.42),
            (left + stroke + size * 0.05, top + (bottom - top) * 0.42),
            (cx + size * 0.20, top),
            (cx + size * 0.20 - stroke * 0.9, top),
        ],
        fill=GLYPH_COLOR,
    )

    # Lower diagonal stroke.
    draw.polygon(
        [
            (left + stroke, top + (bottom - top) * 0.5),
            (left + stroke + size * 0.05, top + (bottom - top) * 0.5),
            (cx + size * 0.22, bottom),
            (cx + size * 0.22 - stroke * 0.9, bottom),
        ],
        fill=GLYPH_COLOR,
    )

    return img


def gen_with_pil():
    master = build_master_pil(1024)
    master.save(os.path.join(HERE, "icon.png"), format="PNG")
    print("wrote icon.png (1024x1024)")

    for s in SIZES:
        resized = master.resize((s, s), Image.LANCZOS)
        name = f"icon-{s}.png"
        resized.save(os.path.join(HERE, name), format="PNG")
        print(f"wrote {name} ({s}x{s})")

    ico_path = os.path.join(HERE, "icon.ico")
    master.save(
        ico_path,
        format="ICO",
        sizes=[(s, s) for s in ICO_SIZES],
    )
    print(f"wrote icon.ico ({', '.join(str(s) for s in ICO_SIZES)})")


# ---------------------------------------------------------------------------
# Pure-python fallback (no PIL): solid RGBA square, no rounded corners/glyph.
# ---------------------------------------------------------------------------

def _png_chunk(tag, data):
    chunk = tag + data
    crc = zlib.crc32(chunk) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + chunk + struct.pack(">I", crc)


def _solid_png_bytes(size, color):
    width = height = size
    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA
    row = bytes(color) * width
    raw = b"".join(b"\x00" + row for _ in range(height))
    idat = zlib.compress(raw, 9)
    png = sig
    png += _png_chunk(b"IHDR", ihdr)
    png += _png_chunk(b"IDAT", idat)
    png += _png_chunk(b"IEND", b"")
    return png


def gen_without_pil():
    master = _solid_png_bytes(1024, BG_COLOR)
    with open(os.path.join(HERE, "icon.png"), "wb") as f:
        f.write(master)
    print("wrote icon.png (1024x1024, solid fallback, no PIL)")

    sizes_bytes = {}
    for s in SIZES:
        data = _solid_png_bytes(s, BG_COLOR)
        sizes_bytes[s] = data
        with open(os.path.join(HERE, f"icon-{s}.png"), "wb") as f:
            f.write(data)
        print(f"wrote icon-{s}.png ({s}x{s}, solid fallback, no PIL)")

    # Hand-rolled ICO container: ICONDIR + ICONDIRENTRY table + PNG blobs
    # (Vista+ ICO format allows PNG-compressed entries directly).
    ico_entries = []
    for s in ICO_SIZES:
        ico_entries.append(sizes_bytes.get(s) or _solid_png_bytes(s, BG_COLOR))

    count = len(ico_entries)
    icondir = struct.pack("<HHH", 0, 1, count)  # reserved, type=1(icon), count

    header_size = 6 + 16 * count
    offset = header_size
    dir_entries = b""
    image_data = b""
    for s, data in zip(ICO_SIZES, ico_entries):
        width_byte = 0 if s >= 256 else s
        height_byte = 0 if s >= 256 else s
        entry = struct.pack(
            "<BBBBHHII",
            width_byte,
            height_byte,
            0,  # color palette
            0,  # reserved
            1,  # color planes
            32,  # bits per pixel
            len(data),
            offset,
        )
        dir_entries += entry
        image_data += data
        offset += len(data)

    with open(os.path.join(HERE, "icon.ico"), "wb") as f:
        f.write(icondir + dir_entries + image_data)
    print(f"wrote icon.ico ({', '.join(str(s) for s in ICO_SIZES)}, solid fallback, no PIL)")


def main():
    if HAVE_PIL:
        print("Pillow detected: generating rounded square + 'K' glyph icons")
        gen_with_pil()
    else:
        print("Pillow NOT found: generating solid-color fallback icons")
        gen_without_pil()


if __name__ == "__main__":
    main()

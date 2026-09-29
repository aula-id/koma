#!/usr/bin/env python3
"""Generate the koma app icon.

A black circle with a winking face (curved stem, a corner that bends
down-right, and a small smile) centered in a rounded white square.
No outline and no shadow. Outside the rounded square is transparent.

Writes the PNG sizes the Debian hicolor install and the shell installer
use, plus icon.ico and icon.icns for Windows and macOS.

Usage:
    python3 assets/gen-placeholder-icons.py

Requires Pillow (PIL). If Pillow is not installed, falls back to hand-rolled
solid-color PNG/ICO generation via zlib/struct.
"""
import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
BG_COLOR = (0xFF, 0xFF, 0xFF, 0xFF)

SIZES = [32, 48, 64, 128, 256, 512]
ICO_SIZES = [16, 32, 48, 64, 128, 256]

try:
    from PIL import Image, ImageDraw

    HAVE_PIL = True
except ImportError:
    HAVE_PIL = False


def _quad(p0, p1, p2, n=72):
    pts = []
    for i in range(n + 1):
        t = i / n
        u = 1 - t
        pts.append((
            u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0],
            u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1],
        ))
    return pts


def _stroke(draw, pts, width, scale):
    """Solid rounded stroke. A single wide line leaves stripes when scaled down."""
    radius = width / 2.0
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        dist = math.hypot(x1 - x0, y1 - y0)
        steps = max(1, int(dist / (width / 8)))
        for i in range(steps + 1):
            t = i / steps
            px = (x0 + (x1 - x0) * t) * scale
            py = (y0 + (y1 - y0) * t) * scale
            r = radius * scale
            draw.ellipse((px - r, py - r, px + r, py + r), fill=(255, 255, 255, 255))


def build_master_pil(size=1024):
    """Wink circle, centered in a rounded white square. No outline, no shadow."""
    scale = 4
    canvas = size * scale
    img = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    radius = int(canvas * 0.22)
    draw.rounded_rectangle((0, 0, canvas - 1, canvas - 1), radius=radius, fill=(255, 255, 255, 255))

    # The approved circle, inset so the white rounded square frames it.
    diameter = canvas * 0.76
    left = (canvas - diameter) / 2
    draw.ellipse((left, left, left + diameter, left + diameter), fill=(0, 0, 0, 255))

    def at(nx, ny):
        return (left + nx * diameter, left + ny * diameter)

    def stroke(p0, p1, p2, width):
        _stroke(draw, [at(*p) for p in _quad(p0, p1, p2)], width * diameter, 1)

    # Curved stem, wink bending down-right, small smile under the wink.
    stroke((0.33, 0.24), (0.50, 0.36), (0.46, 0.55), 0.091)
    stroke((0.645, 0.09), (0.64, 0.18), (0.65, 0.26), 0.084)
    # Fold the lower arm flatter so the corner reads as a blink.
    stroke((0.65, 0.26), (0.74, 0.285), (0.82, 0.30), 0.084)
    stroke((0.55, 0.66), (0.66, 0.73), (0.78, 0.63), 0.067)

    return img.resize((size, size), Image.Resampling.LANCZOS)


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

    icns_path = os.path.join(HERE, "icon.icns")
    try:
        master.save(icns_path, format="ICNS")
        print(f"wrote icon.icns ({master.width}x{master.height})")
    except Exception as exc:
        print(f"warning: Pillow could not write icon.icns: {exc}")
        if os.path.exists(icns_path):
            os.remove(icns_path)


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
        print("Pillow detected: generating wink icon on a rounded white square")
        gen_with_pil()
    else:
        print("Pillow NOT found: generating solid-color fallback icons")
        gen_without_pil()


if __name__ == "__main__":
    main()

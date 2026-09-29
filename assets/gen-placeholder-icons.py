#!/usr/bin/env python3
"""Generate the koma app icon.

A free typewriter "K" (Cousine Bold, falling back to other free mono faces)
centered in a dark circle. Writes the PNG sizes the Debian hicolor install
and the shell installer use, plus icon.ico and icon.icns for Windows and macOS.

Usage:
    python3 assets/gen-placeholder-icons.py

Requires Pillow (PIL). If Pillow is not installed, falls back to hand-rolled
solid-color PNG/ICO generation (no circle, no glyph) via zlib/struct.
"""
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
BG_COLOR = (0x1A, 0x1A, 0x2E, 0xFF)  # #1a1a2e, fully opaque
GLYPH_COLOR = (0xF4, 0xF4, 0xF6, 0xFF)  # light near-white glyph

SIZES = [32, 48, 64, 128, 256, 512]
ICO_SIZES = [16, 32, 48, 64, 128, 256]
# Free typewriter faces, most specific first. Cousine is metric-compatible
# with Courier New and licensed Apache-2.0.
FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/croscore/Cousine-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
    "/usr/share/fonts/truetype/freefont/FreeMonoBold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
]

try:
    from PIL import Image, ImageDraw, ImageFont

    HAVE_PIL = True
except ImportError:
    HAVE_PIL = False


def _font(size):
    for path in FONT_CANDIDATES:
        if os.path.isfile(path):
            return ImageFont.truetype(path, size), path
    return None, None


def build_master_pil(size=1024):
    """Dark circle with a free typewriter 'K' centered on the ink."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    margin = int(size * 0.04)
    draw.ellipse([margin, margin, size - margin - 1, size - margin - 1], fill=BG_COLOR)

    font, font_path = _font(int(size * 0.62))
    if font is None:
        raise RuntimeError("no free font found; install fonts-croscore or fonts-dejavu")

    bbox = font.getbbox("K")
    ink_w = bbox[2] - bbox[0]
    ink_h = bbox[3] - bbox[1]
    x = (size - ink_w) / 2 - bbox[0]
    y = (size - ink_h) / 2 - bbox[1]
    draw.text((x, y), "K", font=font, fill=GLYPH_COLOR)
    print(f"glyph font: {font_path}")
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
        print("Pillow detected: generating circle + typewriter 'K' icons")
        gen_with_pil()
    else:
        print("Pillow NOT found: generating solid-color fallback icons")
        gen_without_pil()


if __name__ == "__main__":
    main()

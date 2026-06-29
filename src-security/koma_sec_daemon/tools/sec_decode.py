"""
sec_decode — decode/transform a string through common CTF encodings.

compute : instant-cpu
risk    : False
domain  : crypto

Pure-Python only (base64, binascii, urllib.parse, codecs, gzip, zlib).
No lazy imports required — all stdlib.
"""

from __future__ import annotations

import base64
import binascii
import codecs
import gzip
import io
import urllib.parse

# Minimum fraction of characters that must be printable for a candidate to be
# accepted in "auto" mode.
_PRINTABLE_THRESHOLD = 0.75


def _is_mostly_printable(s: str) -> bool:
    if not s:
        return False
    printable = sum(1 for c in s if c.isprintable() or c in "\n\r\t")
    return (printable / len(s)) >= _PRINTABLE_THRESHOLD


def _try_base64(data: str) -> str | None:
    # Accept both standard and URL-safe alphabets; add padding if needed.
    for alphabet in (data, data.replace("-", "+").replace("_", "/")):
        padded = alphabet + "=" * (-len(alphabet) % 4)
        try:
            raw = base64.b64decode(padded, validate=True)
            return raw.decode("utf-8", errors="replace")
        except Exception:
            pass
    return None


def _try_base32(data: str) -> str | None:
    padded = data.upper() + "=" * (-len(data) % 8)
    try:
        raw = base64.b32decode(padded)
        return raw.decode("utf-8", errors="replace")
    except Exception:
        return None


def _try_hex(data: str) -> str | None:
    cleaned = data.strip().replace(" ", "").replace("0x", "").replace("\\x", "")
    try:
        raw = binascii.unhexlify(cleaned)
        return raw.decode("utf-8", errors="replace")
    except Exception:
        return None


def _try_url(data: str) -> str | None:
    try:
        result = urllib.parse.unquote(data, errors="strict")
        # Only useful if something actually changed.
        if result != data:
            return result
        return None
    except Exception:
        return None


def _try_rot13(data: str) -> str | None:
    return codecs.encode(data, "rot_13")


def _try_gzip(data: str) -> str | None:
    # Accept raw bytes encoded as base64 first, then decompress.
    for alphabet in (data, data.replace("-", "+").replace("_", "/")):
        padded = alphabet + "=" * (-len(alphabet) % 4)
        try:
            raw = base64.b64decode(padded, validate=True)
            with gzip.GzipFile(fileobj=io.BytesIO(raw)) as gz:
                decompressed = gz.read()
            return decompressed.decode("utf-8", errors="replace")
        except Exception:
            pass
    # Also try treating the input itself as raw gzip bytes (via latin-1 roundtrip).
    try:
        raw = data.encode("latin-1")
        with gzip.GzipFile(fileobj=io.BytesIO(raw)) as gz:
            decompressed = gz.read()
        return decompressed.decode("utf-8", errors="replace")
    except Exception:
        return None


DESCRIPTOR = {
    "name": "sec_decode",
    "description": (
        "Decode or transform a string through common CTF encodings: "
        "base64, base32, hex, URL-encoding, rot13, or gzip. "
        "Use operation='auto' (default) to try all methods and return "
        "every successful candidate labelled by method."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "data": {
                "type": "string",
                "description": "The encoded string to decode (required).",
            },
            "operation": {
                "type": "string",
                "enum": ["auto", "base64", "base32", "hex", "url", "rot13", "gzip"],
                "description": (
                    "Encoding to apply. 'auto' tries all and returns "
                    "every printable result labelled by method (default: auto)."
                ),
                "default": "auto",
            },
        },
        "required": ["data"],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "crypto",
}


def _handler(args: dict, sessions) -> str:
    data = args.get("data", "")
    operation = args.get("operation", "auto")

    if not data:
        return "error: 'data' is required and must not be empty"

    if operation == "base64":
        result = _try_base64(data)
        return result if result is not None else "error: could not decode as base64"

    elif operation == "base32":
        result = _try_base32(data)
        return result if result is not None else "error: could not decode as base32"

    elif operation == "hex":
        result = _try_hex(data)
        return result if result is not None else "error: could not decode as hex"

    elif operation == "url":
        result = _try_url(data)
        return result if result is not None else "error: no URL-encoded sequences found"

    elif operation == "rot13":
        result = _try_rot13(data)
        return result if result is not None else "error: could not apply rot13"

    elif operation == "gzip":
        result = _try_gzip(data)
        return result if result is not None else "error: could not decompress as gzip"

    elif operation == "auto":
        candidates: list[str] = []

        b64 = _try_base64(data)
        if b64 is not None and _is_mostly_printable(b64):
            candidates.append(f"base64 -> {b64}")

        b32 = _try_base32(data)
        if b32 is not None and _is_mostly_printable(b32):
            candidates.append(f"base32 -> {b32}")

        hx = _try_hex(data)
        if hx is not None and _is_mostly_printable(hx):
            candidates.append(f"hex -> {hx}")

        url = _try_url(data)
        if url is not None and _is_mostly_printable(url):
            candidates.append(f"url -> {url}")

        # rot13 always produces printable text — include unconditionally
        r13 = _try_rot13(data)
        if r13 is not None:
            candidates.append(f"rot13 -> {r13}")

        gz = _try_gzip(data)
        if gz is not None and _is_mostly_printable(gz):
            candidates.append(f"gzip -> {gz}")

        if candidates:
            return "\n".join(candidates)
        return "error: could not decode"

    else:
        return f"error: unknown operation '{operation}'"


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler

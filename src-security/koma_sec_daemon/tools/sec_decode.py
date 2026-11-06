"""
sec_decode — decode/transform a string through common CTF encodings.

effect  : static
exec    : cpu
domain  : crypto

Migration reference for phase 2. Every other tool follows this shape:

- `effect` and `exec` replace `risk`/`compute` and must agree with
  `effects.MANIFEST`; the phase-2 metadata validator asserts that, so the
  descriptor and the manifest cannot disagree.
- the handler returns `verdict.typed(...)`, never a bare `error: …` string.
- "it ran and found nothing" is NOT_FOUND; "the call itself was wrong" is ERROR.
  Collapsing the two is the defect the typed contract exists to fix.

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

from koma_sec_daemon import verdict

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
            return None
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
    "effect": "static",
    "exec": "cpu",
    "domain": "crypto",
}

_SINGLE = {
    "base64": (_try_base64, "base64"),
    "base32": (_try_base32, "base32"),
    "hex": (_try_hex, "hex"),
    "url": (_try_url, "url"),
    "rot13": (_try_rot13, "rot13"),
    "gzip": (_try_gzip, "gzip"),
}


def _handler(args: dict, sessions) -> str:
    data = args.get("data", "")
    operation = args.get("operation", "auto")

    if not data:
        return verdict.typed(verdict.NEEDS_INPUT, "'data' is required and must not be empty")

    if operation in _SINGLE:
        fn, label = _SINGLE[operation]
        result = fn(data)
        if result is None:
            return verdict.typed(verdict.NOT_FOUND, f"could not decode as {label}")
        return verdict.typed(verdict.CONFIRMED, result, handle=None)

    if operation != "auto":
        return verdict.typed(verdict.ERROR, f"unknown operation '{operation}'")

    candidates: list[str] = []
    for label, (fn, _name) in _SINGLE.items():
        got = fn(data)
        # rot13 always produces printable text, so it is offered unconditionally.
        if got is None:
            continue
        if label == "rot13" or _is_mostly_printable(got):
            candidates.append(f"{label} -> {got}")

    if not candidates:
        return verdict.typed(verdict.NOT_FOUND, "no known encoding matched")
    return verdict.typed(verdict.CONFIRMED, "\n".join(candidates))


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler

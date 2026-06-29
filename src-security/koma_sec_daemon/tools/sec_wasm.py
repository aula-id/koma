"""
sec_wasm — WebAssembly decompile/disassemble tool using wabt.

compute : instant-cpu
risk    : False
domain  : web-re

wabt binaries (wasm-decompile, wasm2wat) are invoked via sandbox.run().
base64 input is decoded to a temp file, processed, then cleaned up.
No third-party Python imports are needed; stdlib only.
"""

from __future__ import annotations

import base64
import os
import tempfile

DESCRIPTOR = {
    "name": "sec_wasm",
    "description": (
        "Decompile/disassemble a WebAssembly module using wabt "
        "(wasm-decompile or wasm2wat). Pass the .wasm bytes as base64."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "wasm_b64": {
                "type": "string",
                "description": "Base64-encoded bytes of the .wasm file (required).",
            },
            "mode": {
                "type": "string",
                "enum": ["decompile", "wat"],
                "description": "Output mode: 'decompile' (default) uses wasm-decompile; 'wat' uses wasm2wat.",
                "default": "decompile",
            },
        },
        "required": ["wasm_b64"],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "web-re",
}


def _handler(args: dict, sessions) -> str:
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    wasm_b64 = args["wasm_b64"]
    mode = args.get("mode", "decompile")

    # Decode base64 input
    try:
        wasm_bytes = base64.b64decode(wasm_b64)
    except Exception as exc:
        return f"error: failed to decode base64 input: {exc}"

    # Write to a named temp file with .wasm suffix
    wp = None
    try:
        with tempfile.NamedTemporaryFile(suffix=".wasm", delete=False) as tmp:
            tmp.write(wasm_bytes)
            wp = tmp.name

        tool = "wasm-decompile" if mode != "wat" else "wasm2wat"
        output = sandbox.run([tool, wp], timeout=60)
        return output
    except Exception as exc:
        return f"error: {exc}"
    finally:
        if wp is not None:
            try:
                os.unlink(wp)
            except OSError:
                pass


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler

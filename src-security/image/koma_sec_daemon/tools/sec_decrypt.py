"""
sec_decrypt — decrypt a blob with a key or password.

compute : light
risk    : False
domain  : crypto

Local only: the ciphertext and key are both supplied by the caller, so nothing
dials out. Covers the cases a triage turn actually hits — a config-embedded
AES-CBC blob, an XOR-obfuscated string, a base64-wrapped payload — rather than
trying to be a general crypto workbench.

`cryptography` is imported lazily so the registry loads without it. When it is
absent the tool advertises `unavailable` with a hint instead of failing the
handshake.

Every mode reports HOW it read the key (hex, base64, raw text) in the result.
A silent guess turns a wrong-key failure into a confusing padding error, and the
model needs to know which interpretation was tried before it retries.
"""

from __future__ import annotations

import base64
import binascii
import hashlib
import json

# Guards against a runaway: AES-GCM on a multi-megabyte body is fine, but a jailed
# 100 MB blob is not worth decrypting twice.
MAX_BYTES = 8 * 1024 * 1024


def _spawn():
    from koma_sec_daemon import registry

    return registry.spawn_for_thread()


DESCRIPTOR = {
    "name": "sec_decrypt",
    "description": (
        "Decrypt a blob with a key or password: AES-CBC/GCM/CTR with an explicit "
        "key or a password-derived key, or a single-byte/repeating-key XOR. Reports "
        "how the key was interpreted so a wrong guess is visible."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "data": {
                "type": "string",
                "description": "Ciphertext. Interpreted per `data_encoding`.",
            },
            "scheme": {
                "type": "string",
                "enum": ["aes-gcm", "aes-cbc", "aes-ctr", "xor"],
                "description": "Cipher family.",
            },
            "key": {
                "type": "string",
                "description": "Key material. Interpreted per `key_encoding`.",
            },
            "key_encoding": {
                "type": "string",
                "enum": ["hex", "base64", "raw", "password"],
                "description": (
                    "How to read `key`. `password` derives one with PBKDF2 when a "
                    "`salt` is given, else SHA-256 of the text."
                ),
            },
            "data_encoding": {
                "type": "string",
                "enum": ["hex", "base64", "raw"],
                "description": "How to read `data`. Default base64.",
            },
            "iv": {
                "type": "string",
                "description": "IV/nonce, hex or base64 per `key_encoding`. AES only.",
            },
            "salt": {
                "type": "string",
                "description": "PBKDF2 salt for `key_encoding: password`.",
            },
            "iterations": {
                "type": "integer",
                "description": "PBKDF2 iterations for a password. Default 200000.",
            },
            "aad": {
                "type": "string",
                "description": "Additional authenticated data (aes-gcm only).",
            },
        },
        "required": ["data", "scheme", "key"],
    },
    "risk": False,
    "compute": "light",
    "domain": "crypto",
}


def _decode(text: str, encoding: str) -> bytes:
    """Decode caller-supplied bytes, with a useful message on a bad encoding."""
    encoding = (encoding or "").lower()
    if encoding == "hex":
        try:
            return binascii.unhexlify(text.strip().replace(" ", ""))
        except binascii.Error as exc:
            raise ValueError(f"data is not valid hex: {exc}") from exc
    if encoding == "raw":
        return text.encode("latin-1", "replace")
    try:
        return base64.b64decode(text, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise ValueError(f"data is not valid base64: {exc}") from exc


def _derive_key(args: dict) -> tuple[bytes, str]:
    """The key bytes and how they were read, for the report."""
    raw = args["key"]
    encoding = (args.get("key_encoding") or "raw").lower()
    if encoding == "password":
        salt = args.get("salt")
        if salt:
            iterations = int(args.get("iterations") or 200000)
            salt_bytes = _decode(salt, "hex") if _looks_hex(salt) else salt.encode()
            return (
                hashlib.pbkdf2_hmac("sha256", raw.encode(), salt_bytes, iterations),
                f"pbkdf2-sha256, {iterations} iterations, salted",
            )
        # No salt: still deterministic, and saying so matters — an unsalted
        # derivation is a common reason a decryption does not match elsewhere.
        return hashlib.sha256(raw.encode()).digest(), "sha256 of the password, UNSALTED"
    if encoding == "hex":
        return _decode(raw, "hex"), "hex"
    if encoding == "base64":
        return _decode(raw, "base64"), "base64"
    return raw.encode("latin-1", "replace"), "raw text"


def _looks_hex(text: str) -> bool:
    clean = text.strip().replace(" ", "")
    return bool(clean) and len(clean) % 2 == 0 and all(
        c in "0123456789abcdefABCDEF" for c in clean
    )


def _xor(data: bytes, key: bytes) -> bytes:
    if not key:
        raise ValueError("xor needs a non-empty key")
    return bytes(b ^ key[i % len(key)] for i, b in enumerate(data))


def _printable_ratio(blob: bytes) -> float:
    if not blob:
        return 0.0
    ok = sum(1 for b in blob if 9 <= b <= 13 or 32 <= b <= 126)
    return ok / len(blob)


def _handler(args: dict, sessions) -> str:
    try:
        from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes  # noqa: PLC0415
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM  # noqa: PLC0415
    except ImportError:
        return json.dumps(
            {"verdict": "unavailable", "excerpt": "pip install cryptography"}
        )

    scheme = args["scheme"]
    # Decoding failures are the caller's input, not a crash: report them as a
    # verdict so the model sees the same shape as any other outcome.
    try:
        data = _decode(args["data"], args.get("data_encoding") or "base64")
    except ValueError as exc:
        return json.dumps({"verdict": "error", "excerpt": str(exc)})
    if len(data) > MAX_BYTES:
        return json.dumps(
            {
                "verdict": "error",
                "excerpt": f"ciphertext is {len(data)} bytes; cap is {MAX_BYTES}",
            }
        )
    if not data:
        return json.dumps({"verdict": "error", "excerpt": "empty ciphertext"})

    try:
        key, how = _derive_key(args)
        if not key:
            return json.dumps({"verdict": "error", "excerpt": "the key decoded to nothing"})
    except ValueError as exc:
        return json.dumps({"verdict": "error", "excerpt": f"bad key: {exc}"})
    if scheme.startswith("aes") and len(key) not in (16, 24, 32):
        return json.dumps(
            {
                "verdict": "error",
                "excerpt": f"aes needs a 16, 24 or 32 byte key; got {len(key)} ({how})",
            }
        )

    try:
        if scheme == "xor":
            plain = _xor(data, key)
        elif scheme == "aes-gcm":
            try:
                iv = _decode(args.get("iv") or "", "hex")
            except ValueError as exc:
                return json.dumps({"verdict": "error", "excerpt": f"bad iv: {exc}"})
            if not iv:
                return json.dumps({"verdict": "error", "excerpt": "aes-gcm needs an iv/nonce"})
            aad = (args.get("aad") or "").encode() or None
            plain = AESGCM(key).decrypt(iv, data, aad)
        elif scheme in ("aes-cbc", "aes-ctr"):
            try:
                iv = _decode(args.get("iv") or "", "hex")
            except ValueError as exc:
                return json.dumps({"verdict": "error", "excerpt": f"bad iv: {exc}"})
            if not iv:
                return json.dumps({"verdict": "error", "excerpt": f"{scheme} needs an iv"})
            mode = (
                modes.CBC(iv) if scheme == "aes-cbc" else modes.CTR(iv)
            )
            dec = Cipher(algorithms.AES(key), mode).decryptor()
            plain = dec.update(data) + dec.finalize()
            if scheme == "aes-cbc":
                # Strip PKCS#7 by hand: a wrong key usually still finalises, so the
                # padding is the only signal that it did not work.
                pad = plain[-1] if plain else 0
                if not 1 <= pad <= 16 or plain[-pad:] != bytes([pad]) * pad:
                    plain = plain.rstrip(b"\x00")
                    how += " (BORDERLINE: padding did not validate — wrong key or IV?)"
                else:
                    plain = plain[:-pad]
        else:
            return json.dumps({"verdict": "error", "excerpt": f"unknown scheme {scheme}"})
    except Exception as exc:  # noqa: BLE001 - any cipher failure is one verdict
        return json.dumps(
            {
                "verdict": "unproven",
                "excerpt": f"{scheme} failed with this key ({how}): {type(exc).__name__}: {exc}",
            }
        )

    ratio = _printable_ratio(plain)
    # A successful decrypt that yields mostly binary is still a result, but the
    # ratio is reported so the model does not treat garbage as plaintext.
    verdict = "confirmed" if ratio > 0.85 else "unproven"
    text = plain.decode("utf-8", "replace")
    excerpt = f"{len(plain)} bytes, {ratio:.0%} printable, key read as {how}\n\n{text[:4000]}"
    return json.dumps({"verdict": verdict, "excerpt": excerpt})


def handler(args: dict, sessions) -> str:
    return _handler(args, sessions)

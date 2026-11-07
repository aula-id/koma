"""
sec_factor — integer factorization lookup via factordb.com API.

compute : network
risk    : False
domain  : crypto

requests is imported LAZILY inside the handler so the registry loads even
when the package is not installed (import-time safety for the smoke test).
"""

from __future__ import annotations

DESCRIPTOR = {
    "name": "sec_factor",
    "description": "Look up the factorization of an integer via the factordb.com API.",
    "parameters": {
        "type": "object",
        "properties": {
            "n": {
                "type": "string",
                "description": "The integer to factor (required).",
            },
        },
        "required": ["n"],
    },
    "risk": False,
    "compute": "network",
    "domain": "crypto",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without requests installed
    import requests  # noqa: PLC0415

    n = str(args["n"])

    try:
        resp = requests.get(
            "https://factordb.com/api",
            params={"query": n},
            timeout=30,
        )
        data = resp.json()
    except Exception as exc:
        return f"error: factordb lookup failed: {exc}"

    status = data.get("status", "unknown")
    factors = data.get("factors", [])

    # Build a human-readable factor list: base^exp pairs
    if factors:
        parts = []
        for item in factors:
            base, exp = item[0], item[1]
            parts.append(f"{base}^{exp}" if exp != 1 else str(base))
        factor_str = " * ".join(parts)
    else:
        factor_str = "(none returned)"

    return f"status: {status}\nfactors: {factor_str}"


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler

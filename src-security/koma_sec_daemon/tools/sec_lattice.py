"""
sec_lattice — LLL lattice-basis reduction tool.

compute : long-cpu
risk    : False
domain  : crypto

fpylll is imported LAZILY inside the handler so the registry loads even
when the package is not installed (import-time safety for the smoke test).
"""

from __future__ import annotations

DESCRIPTOR = {
    "name": "sec_lattice",
    "description": "LLL-reduce an integer lattice basis using fpylll.",
    "parameters": {
        "type": "object",
        "properties": {
            "matrix": {
                "type": "array",
                "items": {
                    "type": "array",
                    "items": {"type": "integer"},
                },
                "description": "The basis rows as an array of arrays of integers (required).",
            },
        },
        "required": ["matrix"],
    },
    "risk": False,
    "compute": "long-cpu",
    "domain": "crypto",
}


def _handler(args: dict, sessions) -> str:
    # Lazy import — keeps registry loadable without fpylll installed
    try:
        from fpylll import IntegerMatrix, LLL  # noqa: PLC0415
    except ImportError:
        return "error: fpylll not installed"

    matrix = args["matrix"]

    try:
        M = IntegerMatrix.from_matrix(matrix)
        LLL.reduction(M)
        rows = []
        for i in range(M.nrows):
            row = [M[i, j] for j in range(M.ncols)]
            rows.append(" ".join(str(v) for v in row))
        return "\n".join(rows)
    except Exception as exc:
        return f"error: {exc}"


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler

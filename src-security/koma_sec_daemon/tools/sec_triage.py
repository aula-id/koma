"""
sec_triage — static triage of a local binary.

compute : instant-cpu
risk    : False
domain  : pwn

Runs three read-only tools (file, checksec, one_gadget) against the target
binary and returns labelled sections.  No third-party imports at module level
so the registry loads cleanly even when the binaries are absent.
"""

from __future__ import annotations

DESCRIPTOR = {
    "name": "sec_triage",
    "description": (
        "Static triage of a local binary: file type, checksec mitigations, "
        "and one_gadget candidates. Read-only."
    ),
    "parameters": {
        "type": "object",
        "properties": {
            "binary": {
                "type": "string",
                "description": "Path to the binary to triage (required).",
            },
        },
        "required": ["binary"],
    },
    "risk": False,
    "compute": "instant-cpu",
    "domain": "pwn",
}


def _handler(args: dict, sessions) -> str:
    from koma_sec_daemon import sandbox  # noqa: PLC0415

    binary = args["binary"]

    file_out = sandbox.run(["file", binary], timeout=20)
    checksec_out = sandbox.run(["checksec", "--file=" + binary], timeout=20)
    one_gadget_out = sandbox.run(["one_gadget", binary], timeout=60)

    return (
        f"== file ==\n{file_out}\n"
        f"== checksec ==\n{checksec_out}\n"
        f"== one_gadget ==\n{one_gadget_out}"
    )


# Attach handler to descriptor for registry use
DESCRIPTOR["handler"] = _handler

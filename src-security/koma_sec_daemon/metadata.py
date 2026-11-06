"""Descriptor metadata validator, run at handshake time.

`descriptors()` used to ship whatever each tool module declared. A tool whose
metadata was wrong then failed the parent's parse and vanished, with the reason
only in a log the user never sees. Validating here instead has three effects:

- a wrong descriptor is a refusal with the reason attached, listed in the
  handshake reply, rather than a silent absence;
- the manifest in `effects.py` is checked against the descriptor it describes,
  so a row that drifts from its tool is caught at load;
- a missing `effect`/`exec` is caught before the parent has to default it.

Restored tools still carry `risk`/`compute`. A `risk` bool is mapped onto the
effect axis; `compute` maps onto exec where a value has an obvious counterpart.
Neither is trusted to be right — both go through the same checks.
"""

from __future__ import annotations

from koma_sec_daemon import effects

# Old `compute` -> new `exec`, for the restored tools only.
_LEGACY_EXEC = {
    "instant-cpu": effects.CPU,
    "long-cpu": effects.CPU,
    "gpu": effects.CPU,
    "network": effects.NET,
    "executes-target": effects.BINARY,
}


def _legacy_effect(descriptor: dict) -> str:
    """`True` means 'not purely computational'; the manifest row decides how far."""
    return effects.REMOTE if descriptor.get("risk") else effects.LOCAL


def validate(descriptor: dict) -> list[str]:
    """
    Return a list of refusal reasons; empty means the descriptor is shippable.

    Reasons are user-visible, so they name the tool and the field.
    """
    name = descriptor.get("name") or "<unnamed>"
    reasons: list[str] = []

    row = effects.MANIFEST.get(name)
    if row is None:
        return [f"{name}: no effects.MANIFEST row"]

    effect = descriptor.get("effect")
    exec_ = descriptor.get("exec")
    if effect is None and exec_ is None:
        # A restored descriptor: derive, then check the derivation against the
        # manifest so an un-migrated tool still cannot contradict it.
        effect = _legacy_effect(descriptor)
        exec_ = _LEGACY_EXEC.get(descriptor.get("compute", ""))
        if exec_ is None:
            reasons.append(f"{name}: unknown compute class '{descriptor.get('compute')}'")
        if effect != row[0]:
            reasons.append(
                f"{name}: legacy risk maps to {effect}, manifest says {row[0]}"
            )
    if effect is not None and effect not in effects.EFFECTS:
        reasons.append(f"{name}: unknown effect '{effect}'")
    if exec_ is not None and exec_ not in effects.EXECS:
        reasons.append(f"{name}: unknown exec '{exec_}'")

    # The manifest's own axes must match whatever the descriptor claims.
    if effect is not None and effect != row[0]:
        reasons.append(f"{name}: effect '{effect}' disagrees with manifest '{row[0]}'")
    if exec_ is not None and exec_ != row[1]:
        reasons.append(f"{name}: exec '{exec_}' disagrees with manifest '{row[1]}'")

    # A tool that only ever talks to a fixed host (factordb) has no destination
    # parameter by design, so absence is only a fault for the ones that take one.
    if (
        effect in (effects.REMOTE, effects.EXPLOIT)
        and name not in effects.FIXED_HOST
        and not effects.destination_of(_required_hint(descriptor))
    ):
        reasons.append(f"{name}: effect '{effect}' but the schema declares no destination")

    return reasons


def _required_hint(descriptor: dict) -> dict:
    """Synthetic args naming whatever destination the schema's `required` lists."""
    required = (descriptor.get("parameters") or {}).get("required") or []
    for key in required:
        if key in effects.DESTINATION_PARAMS:
            return {key: "placeholder"}
    for key in effects.DESTINATION_PARAMS:
        if key in ((descriptor.get("parameters") or {}).get("properties") or {}):
            return {key: "placeholder"}
    return {}

"""
Tool registry — built by isolated registration, so one broken tool stays one tool.

Every tool module is imported inside its own guard. A module whose third-party
dependency is absent (pwntools, requests, wabt on PATH) contributes a descriptor
marked `unavailable` with an install hint, and the handshake still completes. A
flat `from .tools.x import DESCRIPTOR` block would make any single missing
dependency kill the whole daemon, which is the failure this shape exists to
prevent.

Nothing here installs anything: the hint is text for the parent to show. Running
`pip install` from inside the daemon would block the frame reader and race an
active call.

`unavailable` is separate from `refused`:

- **refused**  — the descriptor disagrees with its manifest. A bug; the tool is
                 hidden because running it would cross the wrong boundary.
- **unavailable** — the descriptor is fine but the dependency is missing. The
                 tool is advertised so the model knows it exists, and a call
                 returns a typed `unavailable` naming the install.

Only `descriptors()` output is wire-safe (handlers stripped).
"""

from __future__ import annotations

import importlib
import threading

from koma_sec_daemon import effects, metadata

# Wire-safe keys — everything except the callable handler.
_WIRE_KEYS = ("name", "description", "parameters", "effect", "exec", "domain")

# Old compute -> new exec, applied when a descriptor has not been migrated yet.
_LEGACY_EXEC = {
    "instant-cpu": effects.CPU,
    "light": effects.CPU,
    "long-cpu": effects.CPU,
    "gpu": effects.CPU,
    "network": effects.NET,
    "executes-target": effects.BINARY,
}

# Module -> the import that decides whether its dependency is present.
# The probe is a module name; a missing one marks the tool unavailable rather
# than refused, because the descriptor itself is sound.
_REQUIRES: dict[str, str] = {
    "http": "requests",
    "remote": "pwn",
    "sec_xss_confirm": "requests",
    "sec_nuclei": "requests",
    "sec_z3": "z3",
    "sec_rop": "pwn",
    "sec_crack": "requests",
    "sec_browser": "playwright",
    "sec_decrypt": "cryptography",
    "sec_xss": "requests",
}

# Actionable text, not a bare name: the model can act on this and the user can
# copy it. Kept short so it does not dominate a descriptor.
_HINTS = {
    "requests": "pip install requests",
    "pwn": "pip install pwntools",
    "z3": "pip install z3-solver",
    "playwright": "pip install playwright && playwright install chromium",
    "cryptography": "pip install cryptography",
}


class Unavailable(Exception):
    """Raised by `call` when a tool's dependency is missing at call time."""

    def __init__(self, tool: str, missing: str) -> None:
        self.tool = tool
        self.missing = missing
        hint = _HINTS.get(missing, f"install {missing}")
        super().__init__(f"{tool}: missing dependency '{missing}' — {hint}")


def _missing(module: str) -> str | None:
    """The missing import for this tool module, or None when it is all present."""
    probe = _REQUIRES.get(module)
    if not probe:
        return None
    try:
        importlib.import_module(probe)
    except ImportError:
        return probe
    return None


def _load(module: str) -> tuple[dict | None, str | None]:
    """
    Import one tool module in isolation.

    Returns `(descriptor, None)` on success, `(descriptor, missing)` when only the
    dependency is absent, and `(None, reason)` when the module itself cannot be
    imported at all (a genuine bug worth surfacing).
    """
    missing = _missing(module)
    if missing:
        # Read the descriptor without importing the module body: it is a literal,
        # so a dependency-gated import inside it would fail too. `DESCRIPTOR` is
        # declared above any dependency use in every tool by convention.
        try:
            desc = importlib.import_module(f"koma_sec_daemon.tools.{module}").DESCRIPTOR
        except Exception:
            return None, f"missing dependency '{missing}'"
        return _with_availability(desc, missing), missing
    try:
        return importlib.import_module(f"koma_sec_daemon.tools.{module}").DESCRIPTOR, None
    except Exception as exc:  # noqa: BLE001 - one bad module must not kill the rest
        return None, f"{type(exc).__name__}: {exc}"


def _with_availability(descriptor: dict, missing: str) -> dict:
    out = dict(descriptor)
    out["available"] = False
    out["missing"] = missing
    out["hint"] = _HINTS.get(missing, f"install {missing}")
    return out


def _modules() -> list[str]:
    """Every tool module, discovered. Handlers ship alongside their descriptor."""
    from pathlib import Path

    here = Path(__file__).parent / "tools"
    return sorted(p.stem for p in here.glob("*.py") if p.stem != "__init__")


def _build() -> tuple[dict[str, dict], list[dict], list[dict]]:
    """(registry, refused, unavailable) — one pass, no shared mutable state."""
    registry: dict[str, dict] = {}
    refused: list[dict] = []
    unavailable: list[dict] = []
    for module in _modules():
        desc, whats_wrong = _load(module)
        if desc is None:
            refused.append({"name": module, "reasons": [f"import failed: {whats_wrong}"]})
            continue
        name = desc.get("name", module)
        normalised = _normalised(desc)
        reasons = metadata.validate(normalised)
        if reasons:
            refused.append({"name": name, "reasons": reasons})
            continue
        # The import this tool needs where it runs (guest, or host for browser).
        normalised["requires_module"] = _REQUIRES.get(module)
        registry[name] = normalised
        if desc.get("available") is False:
            unavailable.append(
                {"name": name, "missing": desc["missing"], "hint": desc["hint"]}
            )
    return registry, refused, unavailable


def _normalised(descriptor: dict) -> dict:
    """The descriptor with capability metadata on the phase-2 vocabulary."""
    out = dict(descriptor)
    if out.get("effect") is None and out.get("exec") is None:
        row = effects.MANIFEST.get(out.get("name", ""))
        # The manifest is the authority for restored tools; the legacy bool only
        # has to agree with it, which `metadata.validate` checks first.
        out["effect"] = row[0] if row else None
        out["exec"] = _LEGACY_EXEC.get(out.get("compute", ""))
    return out


# Built once at import. Cheap: 23 local imports, no third-party import is forced
# (a present dependency is only probed, never re-imported).
REGISTRY, REFUSED, UNAVAILABLE = _build()


def descriptors() -> list[dict]:
    """Wire-safe descriptors of every VALID tool. Unavailable ones are included:
    the model should know they exist and what they need."""
    return [{k: d[k] for k in _WIRE_KEYS if k in d} | _availability(d) for d in REGISTRY.values()]


def _availability(d: dict) -> dict:
    if d.get("available") is False:
        return {"available": False, "missing": d["missing"], "hint": d["hint"]}
    return {}


def refused_descriptors() -> list[dict]:
    """Descriptors rejected by `metadata.validate`, with reasons. A bug list."""
    return REFUSED


def unavailable_tools() -> list[dict]:
    """Valid tools whose dependency is missing, with an install hint."""
    return UNAVAILABLE


# The frame id of the handler currently running, so a spawned child can be
# attributed to the call that started it and killed by that id.
_CURRENT: dict[int, object] = {}


def _pack_row(name: str) -> dict | None:
    """Catalog pack row for *name* (id or display name), or None."""
    try:
        from koma_sec_daemon import catalog
    except Exception:  # noqa: BLE001
        return None
    packs = catalog._load_packs()  # noqa: SLF001 — shared pack index
    if name in packs:
        return packs[name]
    needle = (name or "").strip().lower()
    if not needle:
        return None
    for row in packs.values():
        if not isinstance(row, dict):
            continue
        if str(row.get("id") or "").strip().lower() == needle:
            return row
        if str(row.get("name") or "").strip().lower() == needle:
            return row
    return None


def _generic_from_pack(name: str, args: dict) -> str | None:
    """Run generic_cli for a pack-only tool. None = not a pack tool."""
    import shutil

    row = _pack_row(name)
    if row is None:
        return None
    requires = row.get("requires") or []
    bin_name = None
    if isinstance(requires, list) and requires:
        cand = requires[0]
        if isinstance(cand, str) and cand.strip():
            bin_name = cand.strip()
    if not bin_name:
        bin_name = str(row.get("name") or row.get("id") or name).strip()
    if not bin_name:
        return None
    # Resolve by catalog tool name so pack requires/how_to are used.
    from koma_sec_daemon import generic_cli

    _ = bin_name  # pack binary name; generic_cli re-resolves via catalog
    _ = shutil.which  # keep import used for future fast-path
    return generic_cli.run(name, args or {})

def call(name: str, args: dict, sessions, cancel_key: object | None = None) -> str:
    """
    Look up *name* and invoke its handler.

    Raises KeyError for an unknown tool, `Unavailable` when the dependency is
    missing (so the parent can answer with a typed verdict instead of a crash),
    and lets any handler exception propagate unchanged.

    Pack-only catalog tools (no dedicated handler) fall back to generic_cli
    when a pack row names a binary via `requires`.
    """
    try:
        descriptor = REGISTRY[name]
    except KeyError:
        generic = _generic_from_pack(name, args or {})
        if generic is not None:
            return generic
        raise KeyError(name)
    if descriptor.get("available") is False:
        raise Unavailable(name, descriptor["missing"])
    # Call outside the try so a handler's own KeyError is not mistaken for an
    # unknown tool.
    #
    # Thread-local, not shared: several handlers run at once and each must
    # attribute its children to its OWN frame id.
    if cancel_key is not None:
        from koma_sec_daemon import cancel

        _CURRENT[threading.get_ident()] = cancel.wrap(cancel_key)
    try:
        return descriptor["handler"](args, sessions)
    finally:
        _CURRENT.pop(threading.get_ident(), None)


def spawn_for_thread():
    """The `spawn=` factory for this thread, or None outside a dispatch."""
    return _CURRENT.get(threading.get_ident())

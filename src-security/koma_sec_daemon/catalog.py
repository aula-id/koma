"""Model-facing tool catalog built from the live registry.

The parent advertises four meta-tools; the model discovers concrete tools here
instead of receiving every descriptor on every turn.
"""
from __future__ import annotations

from koma_sec_daemon import effects, registry


def categories() -> list[str]:
    """Sorted unique domains among valid tools."""
    domains = {
        (d.get("domain") or "other").strip().lower() or "other"
        for d in registry.REGISTRY.values()
    }
    return sorted(domains)


def list_tools(
    category: str | None = None,
    filter_mode: str = "all",
) -> list[dict]:
    """
    Summaries for catalog browsing.

    filter_mode:
      - "all": every valid tool (including unavailable deps)
      - "ready": only tools whose dependencies are present
    """
    cat = (category or "").strip().lower() or None
    want_ready = (filter_mode or "all").strip().lower() == "ready"
    out: list[dict] = []
    for name, d in sorted(registry.REGISTRY.items(), key=lambda kv: kv[0]):
        domain = (d.get("domain") or "other").strip().lower() or "other"
        if cat is not None and domain != cat:
            continue
        ready = d.get("available") is not False
        if want_ready and not ready:
            continue
        row = effects.MANIFEST.get(name)
        effect = d.get("effect") or (row[0] if row else None)
        exec_cls = d.get("exec") or (row[1] if row else None)
        desc = (d.get("description") or "").strip()
        blurb = desc.split("\n", 1)[0][:160]
        item = {
            "name": name,
            "domain": domain,
            "blurb": blurb,
            "effect": effect,
            "exec": exec_cls,
            "ready": ready,
        }
        if not ready:
            item["missing"] = d.get("missing")
            item["hint"] = d.get("hint")
        out.append(item)
    return out


def get_tool(name: str) -> dict | None:
    """Full schema + how-to for one tool, or None if unknown/refused."""
    d = registry.REGISTRY.get(name)
    if d is None:
        return None
    row = effects.MANIFEST.get(name)
    effect = d.get("effect") or (row[0] if row else None)
    exec_cls = d.get("exec") or (row[1] if row else None)
    note = row[3] if row else ""
    ready = d.get("available") is not False
    out = {
        "name": name,
        "description": d.get("description") or "",
        "parameters": d.get("parameters") or {"type": "object", "properties": {}},
        "domain": (d.get("domain") or "other").strip().lower() or "other",
        "effect": effect,
        "exec": exec_cls,
        "how_to": note or f"Call via sec_run with name={name!r} and args matching parameters.",
        "ready": ready,
    }
    if not ready:
        out["missing"] = d.get("missing")
        out["hint"] = d.get("hint")
    return out


def handle(op: str, args: dict | None = None) -> dict:
    """Dispatch catalog sub-ops. Returns a JSON-serializable result object."""
    args = args or {}
    op = (op or "").strip().lower()
    if op == "categories":
        return {"categories": categories()}
    if op == "list":
        return {
            "tools": list_tools(
                category=args.get("category"),
                filter_mode=str(args.get("filter") or "all"),
            )
        }
    if op == "get":
        name = args.get("name")
        if not isinstance(name, str) or not name.strip():
            raise ValueError("catalog get requires string 'name'")
        tool = get_tool(name.strip())
        if tool is None:
            raise KeyError(f"unknown tool: {name}")
        return {"tool": tool}
    raise ValueError(f"unknown catalog op: {op!r}")

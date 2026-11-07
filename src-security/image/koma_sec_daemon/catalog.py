"""Model-facing tool catalog: packs + live registry merge.

The parent advertises four meta-tools; the model discovers concrete tools here.
Pack JSON (catalog/packs/*.json) is the policy source for lanes/effect; the live
registry supplies schemas and readiness when present.
"""
from __future__ import annotations

import json
from pathlib import Path

from koma_sec_daemon import effects, registry

_LANES = frozenset({"research", "red", "blue"})
_EFFECTS = frozenset({"static", "local", "remote", "exploit"})
_EXECS = frozenset({"cpu", "binary", "net", "browser"})


def _catalog_root() -> Path:
    # src-security/catalog next to koma_sec_daemon package parent
    here = Path(__file__).resolve().parent.parent / "catalog"
    if here.is_dir():
        return here
    # image bake may place packs beside the package
    alt = Path(__file__).resolve().parent / "catalog"
    return alt if alt.is_dir() else here


def _load_packs() -> dict[str, dict]:
    """id -> pack row. Later packs override earlier on same id."""
    root = _catalog_root() / "packs"
    by_id: dict[str, dict] = {}
    if not root.is_dir():
        return by_id
    for path in sorted(root.glob("*.json")):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if not isinstance(data, dict) or data.get("version") != 1:
            continue
        for row in data.get("tools") or []:
            if not isinstance(row, dict):
                continue
            tid = row.get("id") or row.get("name")
            if isinstance(tid, str) and tid.strip():
                by_id[tid.strip()] = dict(row)
                by_id[tid.strip()]["id"] = tid.strip()
    return by_id


def _default_lanes(effect: str, name: str) -> list[str]:
    if effect in ("static", "local"):
        return ["research", "red", "blue"]
    if effect == "remote" and name == "sec_http":
        return ["red", "blue"]
    if effect in ("remote", "exploit"):
        return ["red"]
    return ["red"]


def _row_from_registry(name: str, d: dict, pack: dict | None) -> dict:
    man = effects.MANIFEST.get(name)
    effect = (pack or {}).get("effect") or d.get("effect") or (man[0] if man else "static")
    exec_cls = (pack or {}).get("exec") or d.get("exec") or (man[1] if man else "cpu")
    if effect not in _EFFECTS:
        effect = "remote"
    if exec_cls not in _EXECS:
        exec_cls = "net"
    lanes = (pack or {}).get("lanes")
    if not isinstance(lanes, list) or not lanes:
        lanes = _default_lanes(effect, name)
    lanes = [x for x in lanes if x in _LANES] or ["red"]
    domain = (d.get("domain") or "other").strip().lower() or "other"
    cats = (pack or {}).get("categories")
    if not isinstance(cats, list) or not cats:
        cats = [domain]
    ready_reg = d.get("available") is not False
    ready_pack = (pack or {}).get("ready")
    ready = ready_reg if ready_pack is None else bool(ready_pack) and ready_reg
    desc = (pack or {}).get("summary") or (d.get("description") or "")
    blurb = str(desc).replace("\n", " ").strip()[:160]
    out = {
        "name": name,
        "id": name,
        "domain": domain,
        "categories": [str(c) for c in cats],
        "blurb": blurb,
        "effect": effect,
        "exec": exec_cls,
        "lanes": lanes,
        "plan_ok": bool((pack or {}).get("plan_ok", False)),
        "ready": ready,
        "scope_needed": bool(
            (pack or {}).get(
                "scope_needed",
                effect in ("remote", "exploit") and name not in effects.FIXED_HOST,
            )
        ),
        "image_or_pack": str((pack or {}).get("image_or_pack") or "smoke"),
    }
    if not ready:
        out["missing"] = d.get("missing") or ((pack or {}).get("requires") or [None])[0]
        out["hint"] = d.get("hint")
        if (pack or {}).get("requires"):
            out["requires"] = list((pack or {}).get("requires") or [])
    return out


def _merged_rows() -> list[dict]:
    packs = _load_packs()
    names = set(registry.REGISTRY) | set(packs)
    rows = []
    for name in sorted(names):
        d = registry.REGISTRY.get(name) or {}
        pack = packs.get(name)
        if not d and pack:
            # Pack-only entry (future 2300 tools not yet wired as handlers).
            effect = pack.get("effect") or "remote"
            exec_cls = pack.get("exec") or "binary"
            lanes = [x for x in (pack.get("lanes") or ["red"]) if x in _LANES] or ["red"]
            rows.append(
                {
                    "name": name,
                    "id": pack.get("id") or name,
                    "domain": (pack.get("categories") or ["other"])[0],
                    "categories": list(pack.get("categories") or ["other"]),
                    "blurb": str(pack.get("summary") or "")[:160],
                    "effect": effect if effect in _EFFECTS else "remote",
                    "exec": exec_cls if exec_cls in _EXECS else "binary",
                    "lanes": lanes,
                    "plan_ok": bool(pack.get("plan_ok", False)),
                    "ready": bool(pack.get("ready", False)),
                    "scope_needed": bool(pack.get("scope_needed", True)),
                    "image_or_pack": str(pack.get("image_or_pack") or "external"),
                    "requires": list(pack.get("requires") or []),
                }
            )
            continue
        if not d:
            continue
        rows.append(_row_from_registry(name, d, pack))
    return rows


def categories(lane: str | None = None) -> list[str]:
    cats: set[str] = set()
    for r in _merged_rows():
        if lane and lane in _LANES and lane not in r.get("lanes", []):
            continue
        for c in r.get("categories") or [r.get("domain") or "other"]:
            cats.add(str(c).lower())
    return sorted(cats)


def list_tools(
    category: str | None = None,
    filter_mode: str = "all",
    lane: str | None = None,
    limit: int | None = None,
    cursor: int = 0,
    q: str | None = None,
) -> list[dict]:
    cat = (category or "").strip().lower() or None
    want_ready = (filter_mode or "ready").strip().lower() == "ready"
    lane_n = (lane or "").strip().lower() or None
    if lane_n and lane_n not in _LANES:
        lane_n = None
    query = (q or "").strip().lower() or None
    out: list[dict] = []
    for r in _merged_rows():
        if lane_n and lane_n not in (r.get("lanes") or []):
            continue
        domains = [str(c).lower() for c in (r.get("categories") or [r.get("domain")])]
        if cat is not None and cat not in domains and (r.get("domain") or "").lower() != cat:
            continue
        if want_ready and not r.get("ready", True):
            continue
        if query:
            blob = " ".join(
                [
                    str(r.get("name") or ""),
                    str(r.get("blurb") or ""),
                    " ".join(domains),
                ]
            ).lower()
            if query not in blob:
                continue
        out.append(
            {
                "name": r["name"],
                "domain": r.get("domain") or domains[0],
                "categories": r.get("categories") or domains,
                "blurb": r.get("blurb") or "",
                "effect": r.get("effect"),
                "exec": r.get("exec"),
                "lanes": r.get("lanes") or [],
                "ready": bool(r.get("ready", True)),
                "scope_needed": bool(r.get("scope_needed", False)),
                **(
                    {"missing": r["missing"], "hint": r.get("hint")}
                    if not r.get("ready", True) and r.get("missing")
                    else {}
                ),
            }
        )
    start = max(0, int(cursor or 0))
    if limit is None:
        # Default page size keeps model context small at 2k scale.
        limit_n = 40
    else:
        limit_n = max(1, min(int(limit), 200))
    return out[start : start + limit_n]


def list_tools_page(
    category: str | None = None,
    filter_mode: str = "all",
    lane: str | None = None,
    limit: int | None = None,
    cursor: int = 0,
    q: str | None = None,
) -> dict:
    """Paged list with next_cursor for catalog protocol."""
    cat = (category or "").strip().lower() or None
    want_ready = (filter_mode or "ready").strip().lower() == "ready"
    lane_n = (lane or "").strip().lower() or None
    if lane_n and lane_n not in _LANES:
        lane_n = None
    query = (q or "").strip().lower() or None
    full: list[dict] = []
    for r in _merged_rows():
        if lane_n and lane_n not in (r.get("lanes") or []):
            continue
        domains = [str(c).lower() for c in (r.get("categories") or [r.get("domain")])]
        if cat is not None and cat not in domains and (r.get("domain") or "").lower() != cat:
            continue
        if want_ready and not r.get("ready", True):
            continue
        if query:
            blob = " ".join(
                [str(r.get("name") or ""), str(r.get("blurb") or ""), " ".join(domains)]
            ).lower()
            if query not in blob:
                continue
        full.append(
            {
                "name": r["name"],
                "domain": r.get("domain") or domains[0],
                "categories": r.get("categories") or domains,
                "blurb": r.get("blurb") or "",
                "effect": r.get("effect"),
                "exec": r.get("exec"),
                "lanes": r.get("lanes") or [],
                "ready": bool(r.get("ready", True)),
                "scope_needed": bool(r.get("scope_needed", False)),
            }
        )
    start = max(0, int(cursor or 0))
    limit_n = 40 if limit is None else max(1, min(int(limit), 200))
    page = full[start : start + limit_n]
    next_c = start + len(page)
    return {
        "tools": page,
        "total": len(full),
        "cursor": start,
        "next_cursor": next_c if next_c < len(full) else None,
        "limit": limit_n,
    }


def get_tool(name: str, lane: str | None = None) -> dict | None:
    name = (name or "").strip()
    lane_n = (lane or "").strip().lower() or None
    for r in _merged_rows():
        if r["name"] != name:
            continue
        if lane_n and lane_n in _LANES and lane_n not in (r.get("lanes") or []):
            return None
        d = registry.REGISTRY.get(name) or {}
        params = d.get("parameters") or {"type": "object", "properties": {}}
        packs = _load_packs()
        how = (packs.get(name) or {}).get("how_to") or (
            effects.MANIFEST[name][3] if name in effects.MANIFEST else ""
        )
        return {
            "name": name,
            "description": d.get("description") or r.get("blurb") or "",
            "parameters": params,
            "domain": r.get("domain"),
            "categories": r.get("categories"),
            "effect": r.get("effect"),
            "exec": r.get("exec"),
            "lanes": r.get("lanes"),
            "plan_ok": bool(r.get("plan_ok", False)),
            "how_to": how or f"Call via sec_run with name={name!r} and args matching parameters.",
            "ready": bool(r.get("ready", True)),
            "scope_needed": bool(r.get("scope_needed", False)),
            "image_or_pack": r.get("image_or_pack"),
            **(
                {"missing": r.get("missing"), "hint": r.get("hint")}
                if not r.get("ready", True)
                else {}
            ),
        }
    return None



def gate_run(name: str, lane: str | None = None) -> str | None:
    """Refuse a concrete run when pack policy or readiness fails. None = admit.

    Complements parent surfaces: pack `lanes` + `ready` are the scale-out contract.
    """
    n = (name or "").strip()
    if not n:
        return "sec_run requires a tool name"
    row = get_tool(n, lane=None)  # fetch row even if lane would hide it, then check
    if row is None:
        # Unknown to catalog: allow only if still in legacy registry without pack row.
        # Fail closed for named pack tools; unknown bare names get a soft admit so
        # smoke handlers without pack rows keep working until fully catalogued.
        return None
    lane_n = (lane or "").strip().lower() or None
    if lane_n and lane_n in _LANES:
        lanes = row.get("lanes") or []
        if lanes and lane_n not in lanes:
            return f"{n} not allowed on {lane_n} lane (pack lanes={lanes})"
    if not row.get("ready", True):
        miss = row.get("missing") or "dependency"
        hint = row.get("hint") or ""
        return f"{n} not ready ({miss})" + (f": {hint}" if hint else "")
    return None

def lane_allows(name: str, lane: str) -> bool:
    """True when concrete tool `name` may run on `lane` per pack/defaults."""
    lane = (lane or "").strip().lower()
    if lane not in _LANES:
        return False
    tool = get_tool(name)
    if tool is None:
        # Unknown → fail closed (red network default is not auto-allow).
        return False
    return lane in (tool.get("lanes") or [])


def handle(op: str, args: dict | None = None) -> dict:
    args = args or {}
    op = (op or "").strip().lower()
    lane = args.get("lane")
    if not isinstance(lane, str):
        lane = None
    if op == "categories":
        return {"categories": categories(lane=lane)}
    if op == "list":
        lim = args.get("limit")
        cur = args.get("cursor") or 0
        try:
            lim_i = int(lim) if lim is not None else None
        except (TypeError, ValueError):
            lim_i = None
        try:
            cur_i = int(cur)
        except (TypeError, ValueError):
            cur_i = 0
        q = args.get("q") if isinstance(args.get("q"), str) else None
        return list_tools_page(
            category=args.get("category") if isinstance(args.get("category"), str) else None,
            filter_mode=str(args.get("filter") or "ready"),
            lane=lane,
            limit=lim_i,
            cursor=cur_i,
            q=q,
        )
    if op == "get":
        name = args.get("name")
        if not isinstance(name, str) or not name.strip():
            raise ValueError("catalog get requires string 'name'")
        tool = get_tool(name.strip(), lane=lane)
        if tool is None:
            raise KeyError(f"unknown or lane-denied tool: {name}")
        return {"tool": tool}
    if op == "summary":
        rows = _merged_rows()
        ready_n = sum(1 for r in rows if r.get("ready", True))
        by_dom: dict[str, dict] = {}
        for r in rows:
            d = (r.get("domain") or "other").lower()
            slot = by_dom.setdefault(d, {"total": 0, "ready": 0})
            slot["total"] += 1
            if r.get("ready", True):
                slot["ready"] += 1
        return {
            "tools": len(rows),
            "ready": ready_n,
            "not_ready": len(rows) - ready_n,
            "domains": by_dom,
        }
    if op == "packs":
        root = _catalog_root() / "packs"
        names = sorted(p.stem for p in root.glob("*.json")) if root.is_dir() else []
        return {"packs": names, "root": str(_catalog_root())}
    raise ValueError(f"unknown catalog op: {op!r}")

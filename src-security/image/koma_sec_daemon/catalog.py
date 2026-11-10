"""Model-facing tool catalog: packs + live registry merge.

The parent advertises four meta-tools; the model discovers concrete tools here.
Pack JSON (catalog/packs/*.json) is the policy source for lanes/effect; the live
registry supplies schemas and readiness when present.
"""
from __future__ import annotations

import json
import os
from pathlib import Path

from koma_sec_daemon import effects, guest_probe, registry

_LANES = frozenset({"research", "red", "blue"})
_EFFECTS = frozenset({"static", "local", "remote", "exploit"})
_EXECS = frozenset({"cpu", "binary", "net", "browser"})
# Pack/image markers that do not require a domain composition match.
_SKIP_IMAGE_GATE = frozenset({"", "smoke", "external", "debian", "none"})
# How long a model-facing op waits for the first guest probe of an image.
PROBE_WAIT_S = 45.0
# The dashboard index blocks the panel, so it waits less and falls back to the pin.
INDEX_WAIT_S = 5.0


def _catalog_root() -> Path:
    # src-security/catalog next to koma_sec_daemon package parent
    here = Path(__file__).resolve().parent.parent / "catalog"
    if here.is_dir():
        return here
    # image bake may place packs beside the package
    alt = Path(__file__).resolve().parent / "catalog"
    return alt if alt.is_dir() else here


def _load_packs() -> dict[str, dict]:
    """id -> pack row. Later packs override earlier on same id.

    `debian.json` is the legacy flat pack. It sorts after the domain shards and
    would reset those rows to `image_or_pack: debian`, which skips the guest
    gate. Domain shards + `minimal.json` + `smoke.json` are the product catalog.
    """
    root = _catalog_root() / "packs"
    by_id: dict[str, dict] = {}
    if not root.is_dir():
        return by_id
    for path in sorted(root.glob("*.json")):
        if path.name == "debian.json":
            continue
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


def _row_from_registry(name: str, d: dict, pack: dict | None, probe) -> dict:
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
    needs = list((pack or {}).get("requires") or [])
    if d.get("requires_module"):
        needs.append(d["requires_module"])
    image = str((pack or {}).get("image_or_pack") or "smoke")
    ready, source, missing, hint, where = _readiness(needs, exec_cls, image, probe)
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
        "ready_source": source,
        "guest_image": where,
        "scope_needed": bool(
            (pack or {}).get(
                "scope_needed",
                effect in ("remote", "exploit") and name not in effects.FIXED_HOST,
            )
        ),
        "image_or_pack": image,
    }
    if not ready:
        out["missing"] = missing
        out["hint"] = hint
        if (pack or {}).get("requires"):
            out["requires"] = list((pack or {}).get("requires") or [])
    return out


def _merged_rows(wait: float = PROBE_WAIT_S) -> list[dict]:
    probe = guests(wait)
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
            requires = list(pack.get("requires") or [])
            image = str(pack.get("image_or_pack") or "external")
            ready_pack, source, missing, hint, where = _readiness(requires, exec_cls, image, probe)
            row = {
                "name": name,
                "id": pack.get("id") or name,
                "domain": (pack.get("categories") or ["other"])[0],
                "categories": list(pack.get("categories") or ["other"]),
                "blurb": str(pack.get("summary") or "")[:160],
                "effect": effect if effect in _EFFECTS else "remote",
                "exec": exec_cls if exec_cls in _EXECS else "binary",
                "lanes": lanes,
                "plan_ok": bool(pack.get("plan_ok", False)),
                "ready": ready_pack,
                "ready_source": source,
                "guest_image": where,
                "scope_needed": bool(pack.get("scope_needed", True)),
                "image_or_pack": image,
                "requires": requires,
                "pack_only": True,
            }
            if not ready_pack:
                row["missing"] = missing
                row["hint"] = hint
            rows.append(row)
            continue
        if not d:
            continue
        rows.append(_row_from_registry(name, d, pack, probe))
    return rows


def _requirement_names() -> frozenset[str]:
    """Every binary or module any catalog tool needs: what the probe asks the guest."""
    names: set[str] = set()
    for row in _load_packs().values():
        for req in row.get("requires") or []:
            if isinstance(req, str) and req.strip():
                names.add(req.strip())
    for d in registry.REGISTRY.values():
        mod = d.get("requires_module")
        if isinstance(mod, str) and mod:
            names.add(mod)
    return frozenset(names)


def active_guests() -> list[tuple[str, str]]:
    """(composition, image ref) for every active guest image, primary first.

    The parent passes the active set as `KOMA_SEC_COMPOSITIONS` / `KOMA_SEC_IMAGES`
    (comma-separated, same order). Without them, the single
    `KOMA_SEC_COMPOSITION` / `KOMA_SEC_IMAGE` pair is the set.
    """
    comps = [c.strip().lower() for c in (os.environ.get("KOMA_SEC_COMPOSITIONS") or "").split(",") if c.strip()]
    images = [i.strip() for i in (os.environ.get("KOMA_SEC_IMAGES") or "").split(",") if i.strip()]
    if comps and len(comps) == len(images):
        seen: set[str] = set()
        out = []
        for comp, image in zip(comps, images):
            if image not in seen:
                seen.add(image)
                out.append((comp, image))
        return out
    return [(_active_composition(), guest_probe.active_image())]


def _active_compositions() -> list[str]:
    return [comp for comp, _ in active_guests()]


def guests(wait: float = 0.0) -> list[tuple[str, str, object]]:
    """(composition, image, probe) per active guest. A probe is None while running.

    Every probe starts at once, then shares one `wait` budget.
    """
    import time

    names = _requirement_names()
    pairs = active_guests()
    for _, image in pairs:
        guest_probe.start(names, image)
    deadline = time.monotonic() + max(0.0, wait)
    out = []
    for comp, image in pairs:
        left = max(0.0, deadline - time.monotonic())
        out.append((comp, image, guest_probe.current(names, wait=left, image=image)))
    return out


def guest(wait: float = 0.0):
    """The primary guest's probe (first active image). Kept for callers of one."""
    return guests(wait)[0][2]


def _readiness(needs: list, exec_cls: str, image: str, probes):
    """(ready, source, missing, hint, guest_image) for one tool.

    Ready means every requirement exists where the tool RUNS: the host for
    browser-class tools, otherwise ANY active guest image. `guest_image` is the
    image ref to run it in: its own composition's image when that has it, else
    the first active image that does. The host PATH never decides a guest tool.

    Until the guests are probed (Docker down, image not pulled, probe still
    running) the image index stands in: the tool is assumed present when its
    image is in the active set.
    """
    needs = [str(n).strip() for n in needs if isinstance(n, str) and n.strip()]
    if exec_cls == effects.BROWSER:
        gone = next((n for n in needs if not guest_probe.local_has(n)), None)
        return gone is None, "host", gone, (f"install {gone} on the host" if gone else None), None
    want = (image or "").strip().lower()
    ordered = sorted(probes, key=lambda g: g[0] != want)
    known = [(c, i, pr) for c, i, pr in ordered if pr is not None and pr.ok]
    for _comp, ref, pr in known:
        if all(pr.has(n) for n in needs):
            return True, "guest", None, None, ref
    unknown = [(c, i, pr) for c, i, pr in ordered if pr is None or not pr.ok]
    names = ", ".join(c for c, _, _ in probes) or "none"
    if known and not unknown:
        gone = next((n for n in needs if not any(pr.has(n) for _, _, pr in known)), needs[0] if needs else image)
        return False, "guest", gone, f"'{gone}' is not in the active guest images ({names})", None
    # Some guest could not be asked: estimate from the active set.
    for comp, ref, _ in ordered:
        if want in _SKIP_IMAGE_GATE or want == comp:
            return True, "index", None, None, ref
    pr = unknown[0][2] if unknown else None
    why = "guest image still being checked" if pr is None else (pr.error or "guest image not checked")
    return False, "index", (needs[0] if needs else image), why, None


def categories(lane: str | None = None) -> list[str]:
    """Every category name in the catalog (ready or not). Internal/tests."""
    cats: set[str] = set()
    for r in _merged_rows(wait=0.0):
        if lane and lane in _LANES and lane not in r.get("lanes", []):
            continue
        for c in r.get("categories") or [r.get("domain") or "other"]:
            cats.add(str(c).lower())
    return sorted(cats)


def ready_categories(lane: str | None = None) -> dict:
    """The model's entry point: categories that have tools ready in the active guests.

    Only ready tools count, so a category appears once an active image can run
    something in it. The model picks one and lists it; it never pages through
    hundreds of tools it cannot run.
    """
    lane_n = (lane or "").strip().lower() or None
    counts: dict[str, int] = {}
    total = 0
    for r in _merged_rows():
        if not r.get("ready", True):
            continue
        if lane_n in _LANES and lane_n not in (r.get("lanes") or []):
            continue
        total += 1
        for c in {str(c).lower() for c in (r.get("categories") or [r.get("domain") or "other"])}:
            counts[c] = counts.get(c, 0) + 1
    ordered = sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))
    return {
        "categories": [{"name": name, "tools": n} for name, n in ordered],
        "ready": total,
        "images": _active_compositions(),
        "next": "sec_catalog op=list category=<name> to see that category's tools",
    }


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
        row_out = {
            "name": r["name"],
            "domain": r.get("domain") or domains[0],
            "categories": r.get("categories") or domains,
            "blurb": r.get("blurb") or "",
            "effect": r.get("effect"),
            "exec": r.get("exec"),
            "lanes": r.get("lanes") or [],
            "ready": bool(r.get("ready", True)),
            "scope_needed": bool(r.get("scope_needed", False)),
            "image_or_pack": r.get("image_or_pack") or "smoke",
        }
        if not r.get("ready", True) and r.get("missing"):
            row_out["missing"] = r["missing"]
            row_out["hint"] = r.get("hint")
        out.append(row_out)
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
                "image_or_pack": r.get("image_or_pack") or "smoke",
                **(
                    {"missing": r.get("missing"), "hint": r.get("hint")}
                    if not r.get("ready", True)
                    else {}
                ),
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
        params = d.get("parameters")
        if not params and r.get("pack_only"):
            params = {
                "type": "object",
                "properties": {
                    "argv": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Argument vector after the binary name",
                    },
                    "raw": {
                        "type": "string",
                        "description": "Shell-style args string (shlex-split); alternative to argv",
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Seconds (default 120, max 600)",
                    },
                },
            }
        if not params:
            params = {"type": "object", "properties": {}}
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
            "image": r.get("image_or_pack"),
            "ready_on_active": bool(r.get("ready", True)),
            "ready_source": r.get("ready_source"),
            "guest_image": r.get("guest_image"),
            **(
                {"missing": r.get("missing"), "hint": r.get("hint")}
                if not r.get("ready", True)
                else {}
            ),
        }
    return None



def _comp_base(s: str) -> str:
    """Domain base of a composition id (`recon-ext` → `recon`)."""
    s = (s or "").strip().lower()
    if not s:
        return ""
    return s.split("-", 1)[0]


def _active_composition() -> str:
    """Guest composition the daemon was started with (parent sets env)."""
    for key in ("KOMA_SEC_COMPOSITION", "KOMA_SEC_ACTIVE_IMAGE"):
        val = (os.environ.get(key) or "").strip().lower()
        if val:
            return val
    return "minimal"


def _composition_satisfies(required: str | None, active: str | None) -> bool:
    """True when the active guest image covers the tool's required composition.

    Skip gate for smoke/external/debian/empty. Otherwise allow when the domain
    base matches (`recon-ext` satisfies `recon`), exact id matches, or active
    is the special `full` composition.
    """
    req_raw = (required or "").strip().lower()
    act = (active or "").strip().lower() or "minimal"
    if act == "full":
        return True
    # Legacy multi-value fields ("minimal,standard") — any token may satisfy.
    parts = [p.strip() for p in req_raw.split(",") if p.strip()] or [""]
    for req in parts:
        if req in _SKIP_IMAGE_GATE:
            return True
        if req == act:
            return True
        if _comp_base(req) and _comp_base(req) == _comp_base(act):
            return True
    return False


def gate_run(name: str, lane: str | None = None) -> str | None:
    """Refuse a concrete run when pack policy or readiness fails. None = admit.

    Complements parent surfaces: pack `lanes` + `ready` are the scale-out contract.
    Also refuses when the active guest composition does not cover `image_or_pack`.
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
    required = row.get("image_or_pack") or ""
    actives = _active_compositions()
    active = ", ".join(actives)
    if row.get("ready") and row.get("ready_source") in ("guest", "host"):
        # A guest (or the host, for browser tools) was asked and has it.
        return None
    req_s = str(required) if required is not None else ""
    if not any(_composition_satisfies(req_s, comp) for comp in actives):
        # Prefer a single composition token for the message (first non-skip part).
        need = str(required).strip()
        for part in (p.strip() for p in need.split(",")):
            if part and part.lower() not in _SKIP_IMAGE_GATE:
                need = part
                break
        return (
            f"{n} needs guest image '{need}' (active='{active}') "
            f"— pull/activate in /security → Images"
        )
    if not row.get("ready", True):
        miss = row.get("missing") or "dependency"
        hint = row.get("hint") or ""
        img = row.get("image_or_pack") or ""
        msg = f"{n} not ready ({miss})"
        extras: list[str] = []
        if hint:
            extras.append(str(hint))
        img_s = str(img)
        # Only point at Images when another image would help; when the active
        # one is meant to ship it, the hint already says it is not in there.
        if img_s and img_s.lower() not in _SKIP_IMAGE_GATE and img_s.lower() not in actives:
            extras.append(
                f"needs guest image '{img_s}' — pull/activate in /security → Images "
                f"(koma-safety:…-{img_s})"
            )
        if extras:
            msg = msg + ": " + "; ".join(extras)
        return msg
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


def index_rows() -> list[dict]:
    """Compact dashboard rows from pack files. No `how_to`, never the host PATH.

    `ready` is present once the active guest has been probed; until then the
    panel overlays readiness from the active pin + local images.
    """
    rows: list[dict] = []
    # The panel loads this once per daemon start; a cached probe answers at once.
    probes = guests(INDEX_WAIT_S)
    probed = all(pr is not None and pr.ok for _, _, pr in probes)
    for tid, row in sorted(_load_packs().items()):
        img = str(row.get("image_or_pack") or "")
        if img.lower() == "debian":
            continue
        domain = _comp_base(img) or str((row.get("categories") or ["other"])[0])
        out = {
            "id": tid,
            "blurb": str(row.get("summary") or "")[:160],
            "image": img,
            "exec": str(row.get("exec") or ""),
            "domain": domain,
        }
        if probed:
            # What the active guests can actually run, so the panel agrees with
            # what the model sees from `sec_catalog`.
            needs = list(row.get("requires") or [])
            d = registry.REGISTRY.get(tid) or {}
            if d.get("requires_module"):
                needs.append(d["requires_module"])
            ready = _readiness(needs, str(row.get("exec") or ""), img, probes)[0]
            out["ready"] = ready
        rows.append(out)
    return rows


def handle(op: str, args: dict | None = None) -> dict:
    args = args or {}
    op = (op or "").strip().lower()
    lane = args.get("lane")
    if not isinstance(lane, str):
        lane = None
    if op in ("", "categories"):
        return ready_categories(lane=lane)
    if op == "list":
        filter_mode = str(args.get("filter") or "ready")
        has_scope = any(isinstance(args.get(k), str) and args.get(k).strip() for k in ("category", "q"))
        if filter_mode == "ready" and not has_scope:
            # No category chosen yet: answer with the categories, not a wall of tools.
            out = ready_categories(lane=lane)
            out["hint"] = "pick a category (or pass q) to list tools"
            return out
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
            filter_mode=filter_mode,
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
        rows = _merged_rows(wait=0.0)
        probes = guests(0.0)
        failed = [pr for _, _, pr in probes if pr is not None and not pr.ok]
        if any(pr is None for _, _, pr in probes):
            source = "checking"
        elif failed:
            source = "index"
        else:
            source = "guest"
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
            "image": guest_probe.active_image(),
            "composition": _active_composition(),
            "images": _active_compositions(),
            "source": source,
            **({"probe_error": failed[0].error} if failed else {}),
        }
    if op == "index":
        rows = index_rows()
        return {"tools": rows, "total": len(rows)}
    if op == "packs":
        root = _catalog_root() / "packs"
        names = (
            sorted(p.stem for p in root.glob("*.json") if p.name != "debian.json")
            if root.is_dir()
            else []
        )
        return {"packs": names, "root": str(_catalog_root())}
    raise ValueError(f"unknown catalog op: {op!r}")

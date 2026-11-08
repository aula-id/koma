#!/usr/bin/env python3
"""Generate catalog pack JSON from install_recipes/recipes.jsonl."""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
RECIPES = ROOT / "recipes.jsonl"
OUT_DIR = ROOT.parent / "catalog" / "packs"


def main() -> int:
    rows = []
    for line in RECIPES.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        rows.append(json.loads(line))

    by_pack: dict[str, list] = {"minimal": []}  # domain packs are bucket-generated; no standard/extended
    for r in rows:
        method = r.get("method") or "skip"
        ready = method != "skip"
        tool = {
            "id": r["slug"].replace("-", "_") if not r["slug"].startswith("sec_") else r["slug"],
            "name": r.get("binary_name") or r["slug"],
            "summary": r.get("license_note") or f"{r['slug']} ({method})",
            "categories": r.get("categories") or ["other"],
            "effect": r["effect"],
            "exec": r["exec"],
            "lanes": r["lanes"],
            "plan_ok": False,
            "ready": ready,
            "requires": [] if ready else [r.get("detect") or r["slug"]],
            "scope_needed": r["effect"] in ("remote", "exploit"),
            "image_or_pack": ",".join(r.get("packs") or ["minimal"]),
            "how_to": f"detect={r.get('detect')}; method={method}; source={r.get('source','')}",
        }
        # Prefer sec_ names when slug already is a daemon tool
        if r["slug"].startswith("sec_"):
            tool["id"] = r["slug"]
            tool["name"] = r["slug"]
        for pack in r.get("packs") or ["minimal"]:
            if pack in by_pack:
                by_pack[pack].append(dict(tool))

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for pack, tools in by_pack.items():
        # de-dupe by id
        seen = set()
        uniq = []
        for t in tools:
            if t["id"] in seen:
                continue
            seen.add(t["id"])
            uniq.append(t)
        path = OUT_DIR / f"{pack}.json"
        path.write_text(
            json.dumps({"version": 1, "pack": pack, "tools": uniq}, indent=2) + "\n",
            encoding="utf-8",
        )
        print(f"wrote {path} ({len(uniq)} tools)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

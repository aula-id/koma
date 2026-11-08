#!/usr/bin/env python3
"""Ingest pentest_tools_cli_debian_linux.json → packs + apt lists + detect bins."""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SRC = ROOT / "pentest_tools_cli_debian_linux.json"
PACK_OUT = ROOT.parent / "catalog" / "packs" / "debian.json"
CHANNEL_ORDER = ("apt", "snap", "go", "cargo", "npm", "from-source")
ID_RE = re.compile(r"^[a-z][a-z0-9_]*$")
APT_SKIP = frozenset(
    {
        "sudo",
        "apt-get",
        "apt",
        "install",
        "update",
        "-y",
        "--yes",
        "-qq",
        "--no-install-recommends",
        "&&",
        "|",
        ";",
        "export",
        "cd",
        "git",
        "clone",
        "make",
        "build-essential",
    }
)


def sanitize_id(raw: str) -> str:
    s = (raw or "").strip().lower().replace("-", "_")
    s = re.sub(r"[^a-z0-9_]", "_", s)
    s = re.sub(r"_+", "_", s).strip("_")
    if not s:
        s = "tool"
    if s[0].isdigit():
        s = "t_" + s
    if not ID_RE.match(s):
        s = "t_" + re.sub(r"[^a-z0-9_]", "", s) or "t_tool"
        if s[0].isdigit():
            s = "t_" + s
    return s


def classify(category: str, category_name: str) -> tuple[str, str, list[str]]:
    blob = f"{category} {category_name}".casefold()
    if any(
        k in blob
        for k in (
            "exploit",
            "xss",
            "sql injection",
            "web application exploitation",
            "vulnerabilit",
        )
    ):
        if "web" in blob or "xss" in blob or "sql" in blob or "exploit" in blob:
            return "exploit", "net", ["red"]
    if any(
        k in blob
        for k in (
            "recon",
            "osint",
            "information",
            "gathering",
            "network",
            "enum",
            "scan",
        )
    ):
        return "remote", "net", ["red", "research"]
    if any(k in blob for k in ("crack", "hash", "cryptanalysis", "password")):
        return "local", "cpu", ["research", "red", "blue"]
    if any(k in blob for k in ("reverse", "forensic", "binary exploitation", "code analysis")):
        return "static", "binary", ["research", "red", "blue"]
    return "remote", "net", ["red"]


def parse_apt_packages(channel: dict) -> list[str]:
    out: list[str] = []
    pkgs = channel.get("packages")
    if isinstance(pkgs, list):
        for p in pkgs:
            if isinstance(p, str) and p.strip() and not p.startswith("#"):
                out.append(p.strip().split("/")[0])
    for cmd in channel.get("commands") or []:
        if not isinstance(cmd, str) or cmd.lstrip().startswith("#"):
            continue
        if "apt" not in cmd and "apt-get" not in cmd:
            continue
        # crude tokenize
        for tok in re.split(r"[\s\\]+", cmd):
            t = tok.strip().strip("'\"")
            if not t or t in APT_SKIP or t.startswith("-") or t.startswith("#"):
                continue
            if t.startswith("http") or "=" in t and not t[0].isalnum():
                continue
            if re.match(r"^[a-z0-9][a-z0-9+._-]*$", t, re.I):
                if t.lower() not in ("apt-get", "apt", "install", "update"):
                    out.append(t)
    # de-dupe preserve order
    seen = set()
    uniq = []
    for p in out:
        k = p.lower()
        if k in seen:
            continue
        seen.add(k)
        uniq.append(p)
    return uniq


def preferred_channel(install: dict) -> tuple[str, str]:
    channels = install.get("channels") or {}
    order = install.get("preferred_order") or list(CHANNEL_ORDER)
    for ch in order:
        if ch in channels and isinstance(channels[ch], dict):
            cmds = channels[ch].get("commands") or []
            primary = install.get("primary_command") or ""
            if primary and ch == (order[0] if order else ""):
                return ch, str(primary)
            for c in cmds:
                if isinstance(c, str) and c.strip() and not c.lstrip().startswith("#"):
                    return ch, c.strip()
            return ch, str(install.get("primary_command") or "")
    for ch in CHANNEL_ORDER:
        if ch in channels:
            return preferred_channel(
                {
                    **install,
                    "preferred_order": [ch],
                }
            )
    return "skip", str(install.get("primary_command") or "")


def main() -> int:
    if not SRC.is_file():
        print(f"missing source: {SRC}", file=sys.stderr)
        return 1
    data = json.loads(SRC.read_text(encoding="utf-8"))
    tools_in = data.get("tools") or []
    used_ids: dict[str, int] = {}
    pack_tools: list[dict] = []
    apt_high: set[str] = set()
    apt_medium: set[str] = set()
    detect_bins: list[str] = []
    conf_counts = {"high": 0, "medium": 0, "other": 0}
    channel_hits: dict[str, int] = {}

    for t in tools_in:
        if not isinstance(t, dict):
            continue
        raw_id = str(t.get("id") or t.get("name") or "tool")
        base = sanitize_id(raw_id)
        n = used_ids.get(base, 0)
        used_ids[base] = n + 1
        tid = base if n == 0 else f"{base}_{n + 1}"
        if not ID_RE.match(tid):
            tid = sanitize_id(tid)

        bin_name = str(t.get("bin") or raw_id).strip() or tid
        cat = str(t.get("category") or "other")
        cat_name = str(t.get("category_name") or cat)
        effect, exec_cls, lanes = classify(cat, cat_name)
        install = t.get("install") if isinstance(t.get("install"), dict) else {}
        conf = str(install.get("confidence") or "medium").lower()
        conf_counts[conf if conf in conf_counts else "other"] = (
            conf_counts.get(conf if conf in conf_counts else "other", 0) + 1
        )

        ch, cmd = preferred_channel(install)
        channel_hits[ch] = channel_hits.get(ch, 0) + 1

        apt_ch = (install.get("channels") or {}).get("apt")
        if isinstance(apt_ch, dict):
            pkgs = parse_apt_packages(apt_ch)
            apt_conf = str(apt_ch.get("confidence") or conf).lower()
            if conf == "high" or apt_conf == "high":
                apt_high.update(pkgs)
            else:
                # medium / other confidence with an apt channel
                apt_medium.update(pkgs)

        desc = str(t.get("description") or t.get("name") or tid).replace("\n", " ").strip()
        pack_tools.append(
            {
                "id": tid,
                "name": tid,
                "summary": desc[:240],
                "categories": [cat.replace(" ", "-").lower() or "other"],
                "effect": effect,
                "exec": exec_cls,
                "lanes": lanes,
                "plan_ok": False,
                "ready": False,
                "requires": [bin_name],
                "scope_needed": effect in ("remote", "exploit"),
                "image_or_pack": "debian",
                "how_to": f"channel={ch}; primary={cmd[:180]}; detect={bin_name}",
            }
        )
        detect_bins.append(bin_name)

    # medium apt should not duplicate high
    apt_medium -= apt_high

    PACK_OUT.parent.mkdir(parents=True, exist_ok=True)
    PACK_OUT.write_text(
        json.dumps({"version": 1, "pack": "debian", "tools": pack_tools}, indent=2) + "\n",
        encoding="utf-8",
    )

    def write_lines(path: Path, items: list[str] | set[str]) -> None:
        lines = sorted({x.strip() for x in items if x and str(x).strip()})
        path.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")

    write_lines(ROOT / "apt_packages_high.txt", apt_high)
    write_lines(ROOT / "apt_packages_medium.txt", apt_medium)
    # unique detect bins preserve first-seen order
    seen_b: set[str] = set()
    bins_u: list[str] = []
    for b in detect_bins:
        if b in seen_b:
            continue
        seen_b.add(b)
        bins_u.append(b)
    (ROOT / "detect_bins.txt").write_text("\n".join(bins_u) + "\n", encoding="utf-8")

    summary = {
        "tools_in": len(tools_in),
        "tools_out": len(pack_tools),
        "apt_high": len(apt_high),
        "apt_medium": len(apt_medium),
        "detect_bins": len(bins_u),
        "confidence": conf_counts,
        "preferred_channels": channel_hits,
        "pack": str(PACK_OUT),
    }
    (ROOT / "debian_summary.json").write_text(
        json.dumps(summary, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())

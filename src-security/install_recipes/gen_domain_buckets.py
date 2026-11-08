#!/usr/bin/env python3
"""Shard pentest_tools_cli_debian_linux.json into domain bake buckets + packs.

Phase 0 of cyber-domain compositions (≤9GB). Does not touch smoke.json.
"""
from __future__ import annotations

import json
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SRC = ROOT / "pentest_tools_cli_debian_linux.json"
BUCKETS = ROOT / "buckets"
PACKS = ROOT.parent / "catalog" / "packs"
SUMMARY_OUT = ROOT / "buckets_summary.json"
COMPOSITIONS_OUT = BUCKETS / "compositions.json"
COMPOSITIONS_MD = ROOT / "COMPOSITIONS.md"

ID_RE = re.compile(r"^[a-z][a-z0-9_]*$")
APT_MAX = 75
SRC_MAX = 40
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

# category slug → domain (explicit map; unmapped → lab)
DOMAIN_ORDER = (
    "recon",
    "web",
    "identity",
    "cloud",
    "rev",
    "crypto",
    "pxn",
    "dfir",
    "lab",
)

CATEGORY_TO_DOMAIN: dict[str, str] = {
    # recon
    "informations-gathering": "recon",
    "osint-and-reconnaissance": "recon",
    "reconnaissance": "recon",
    "networking": "recon",
    "network": "recon",
    # web
    "web-application-exploitation": "web",
    "cms": "web",
    "bug-bounty": "web",
    "vulnerabilities": "web",
    "vulnerability-assessment": "web",
    "fuzzing": "web",
    # identity
    "auth-perms": "identity",
    "cracking": "identity",
    # cloud
    "cloud-services": "cloud",
    "cloud": "cloud",
    # rev
    "reverse-engineering": "rev",
    "code-analysis": "rev",
    "binary-exploitation": "rev",
    "mobile": "rev",
    "operating-systems": "rev",
    # crypto
    "cryptanalysis": "crypto",
    "cryptography": "crypto",
    "steganography": "crypto",
    # pxn
    "system-exploitation": "pxn",
    "red-teaming": "pxn",
    "adversary-simulation": "pxn",
    "wireless": "pxn",
    "honeypot-and-decoy": "pxn",
    "hardware": "pxn",
    "configuration-audit": "pxn",
    # dfir
    "digital-forensics": "dfir",
    "incident-response": "dfir",
    "defensive": "dfir",
    "threat-intelligence": "dfir",
    "crisis-management": "dfir",
    # lab examples (everything else also falls through)
    "development": "lab",
    "other": "lab",
    "databases": "lab",
    "virtualization": "lab",
    "ai-skills": "lab",
    "ai": "lab",
    "artificial-intelligence": "lab",
    "plugins": "lab",
    "source-code-management": "lab",
    "scm": "lab",
}

DOMAIN_BLURB = {
    "recon": "OSINT, network recon, scanning, enumeration",
    "web": "Web app exploitation, CMS, bug bounty, vuln assessment",
    "identity": "Auth, permissions, password cracking",
    "cloud": "Cloud services and cloud-native security",
    "rev": "Reverse engineering, binary/code analysis, mobile",
    "crypto": "Cryptanalysis, cryptography, steganography",
    "pxn": "System exploitation, red team, wireless, adversary sim",
    "dfir": "Digital forensics, IR, defensive, threat intel",
    "lab": "Lab/dev utilities, databases, virtualization, other",
    "minimal": "Daemon plus the handler binaries no domain image bakes",
}

CHANNEL_FILES = ("apt", "go", "npm", "cargo", "src")


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
        if not s or s[0].isdigit():
            s = "t_" + (s or "tool")
    return s


def classify(category: str, category_name: str) -> tuple[str, str, list[str]]:
    """effect / exec / lanes — same heuristics as ingest_debian_linux.py."""
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


def domain_for(category: str) -> str:
    cat = (category or "other").strip().lower().replace(" ", "-")
    if cat in CATEGORY_TO_DOMAIN:
        return CATEGORY_TO_DOMAIN[cat]
    if cat.startswith("ai"):
        return "lab"
    return "lab"


def normalize_channel(ch: str) -> str:
    c = (ch or "").strip().lower()
    if c in ("source", "from-source", "from_source"):
        return "from-source"
    return c


def preferred_channel(install: dict) -> str:
    order = install.get("preferred_order") or []
    if order:
        return normalize_channel(str(order[0]))
    channels = install.get("channels") or {}
    for ch in ("apt", "go", "cargo", "npm", "from-source"):
        if ch in channels or (ch == "from-source" and "source" in channels):
            return ch
    return "skip"


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
        for tok in re.split(r"[\s\\]+", cmd):
            t = tok.strip().strip("'\"")
            if not t or t in APT_SKIP or t.startswith("-") or t.startswith("#"):
                continue
            if t.startswith("http") or ("=" in t and not t[0].isalnum()):
                continue
            if re.match(r"^[a-z0-9][a-z0-9+._-]*$", t, re.I):
                if t.lower() not in ("apt-get", "apt", "install", "update"):
                    out.append(t)
    seen: set[str] = set()
    uniq: list[str] = []
    for p in out:
        k = p.lower()
        if k in seen:
            continue
        seen.add(k)
        uniq.append(p)
    return uniq


def go_install_spec(channel: dict, primary: str) -> str | None:
    candidates: list[str] = []
    if primary and "go install" in primary:
        candidates.append(primary)
    for cmd in channel.get("commands") or []:
        if isinstance(cmd, str) and not cmd.lstrip().startswith("#"):
            candidates.append(cmd)
    mod = channel.get("module")
    if isinstance(mod, str) and mod.strip():
        m = mod.strip()
        return m if "@" in m else f"{m}@latest"
    for cmd in candidates:
        m = re.search(r"go\s+install\s+(\S+)", cmd)
        if m:
            return m.group(1).strip().strip("'\"")
    return None


def npm_package(channel: dict, primary: str) -> str | None:
    pkg = channel.get("package")
    if isinstance(pkg, str) and pkg.strip():
        return pkg.strip()
    for cmd in [primary, *(channel.get("commands") or [])]:
        if not isinstance(cmd, str) or cmd.lstrip().startswith("#"):
            continue
        m = re.search(r"npm\s+(?:i|install)\s+(?:-g\s+)?(\S+)", cmd)
        if m:
            return m.group(1).strip().strip("'\"")
    return None


def cargo_package(channel: dict, primary: str) -> str | None:
    pkg = channel.get("package")
    if isinstance(pkg, str) and pkg.strip():
        return pkg.strip()
    for cmd in [primary, *(channel.get("commands") or [])]:
        if not isinstance(cmd, str) or cmd.lstrip().startswith("#"):
            continue
        m = re.search(r"cargo\s+install\s+(\S+)", cmd)
        if m:
            return m.group(1).strip().strip("'\"")
    return None


def src_spec(channel: dict, tool: dict) -> str | None:
    repo = channel.get("repository") or tool.get("repository")
    if isinstance(repo, str) and repo.strip():
        return repo.strip()
    tid = tool.get("id") or tool.get("name")
    if tid:
        return str(tid).strip()
    return None


def write_lines(path: Path, items: list[str]) -> None:
    # preserve first-seen order, unique
    seen: set[str] = set()
    lines: list[str] = []
    for x in items:
        s = str(x).strip()
        if not s or s.startswith("#"):
            continue
        key = s.lower() if not s.startswith("http") else s
        if key in seen:
            continue
        seen.add(key)
        lines.append(s)
    path.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")


def empty_bucket_set(comp_id: str) -> dict[str, list[str]]:
    return {ch: [] for ch in CHANNEL_FILES} | {"bins": []}


def pack_tool_row(
    tool: dict,
    tid: str,
    comp_id: str,
    ch: str,
    primary: str,
    bin_name: str,
) -> dict:
    cat = str(tool.get("category") or "other")
    cat_name = str(tool.get("category_name") or cat)
    effect, exec_cls, lanes = classify(cat, cat_name)
    desc = str(tool.get("description") or tool.get("name") or tid).replace("\n", " ").strip()
    return {
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
        "image_or_pack": comp_id,
        "how_to": f"channel={ch}; primary={primary[:180]}; detect={bin_name}",
    }


def main() -> int:
    if not SRC.is_file():
        print(f"missing source: {SRC}", file=sys.stderr)
        return 1

    data = json.loads(SRC.read_text(encoding="utf-8"))
    tools_in = data.get("tools") or []

    # domain → channel → list of (tool, install_meta)
    by_domain: dict[str, dict[str, list[dict]]] = {
        d: defaultdict(list) for d in DOMAIN_ORDER
    }
    skipped_snap = 0
    skipped_other = 0
    category_counts: dict[str, int] = defaultdict(int)
    domain_counts: dict[str, int] = defaultdict(int)

    for t in tools_in:
        if not isinstance(t, dict):
            continue
        cat = str(t.get("category") or "other")
        domain = domain_for(cat)
        category_counts[cat] += 1
        domain_counts[domain] += 1
        install = t.get("install") if isinstance(t.get("install"), dict) else {}
        ch = preferred_channel(install)
        if ch == "snap":
            skipped_snap += 1
            continue
        if ch not in ("apt", "go", "npm", "cargo", "from-source"):
            skipped_other += 1
            continue
        channels = install.get("channels") or {}
        # normalize source key
        if ch == "from-source" and "from-source" not in channels and "source" in channels:
            channels = {**channels, "from-source": channels["source"]}
        ch_body = channels.get(ch) if isinstance(channels.get(ch), dict) else {}
        primary = str(install.get("primary_command") or "")
        bin_name = str(t.get("bin") or t.get("id") or t.get("name") or "tool").strip()
        entry = {
            "tool": t,
            "channel": ch,
            "channel_body": ch_body or {},
            "primary": primary,
            "bin": bin_name,
            "domain": domain,
        }
        by_domain[domain][ch].append(entry)

    compositions: list[dict] = []
    # comp_id → bucket channel lists + pack tools
    buckets: dict[str, dict[str, list]] = {}
    pack_tools: dict[str, list[dict]] = {}
    used_ids_global: dict[str, int] = {}

    def alloc_id(raw: str) -> str:
        base = sanitize_id(raw)
        n = used_ids_global.get(base, 0)
        used_ids_global[base] = n + 1
        tid = base if n == 0 else f"{base}_{n + 1}"
        if not ID_RE.match(tid):
            tid = sanitize_id(tid)
        return tid

    def ensure_comp(comp_id: str) -> None:
        if comp_id not in buckets:
            buckets[comp_id] = empty_bucket_set(comp_id)
            pack_tools[comp_id] = []

    def add_tool_to_comp(comp_id: str, entry: dict, bake_channel: str, bake_value: str | None) -> None:
        ensure_comp(comp_id)
        t = entry["tool"]
        raw_id = str(t.get("id") or t.get("name") or "tool")
        tid = alloc_id(raw_id)
        bin_name = entry["bin"]
        primary = entry["primary"]
        ch = entry["channel"]
        buckets[comp_id]["bins"].append(bin_name)
        if bake_value:
            file_key = "src" if bake_channel == "from-source" else bake_channel
            if file_key in buckets[comp_id]:
                buckets[comp_id][file_key].append(bake_value)
        pack_tools[comp_id].append(
            pack_tool_row(t, tid, comp_id, ch, primary, bin_name)
        )

    for domain in DOMAIN_ORDER:
        chans = by_domain[domain]
        apt_entries = list(chans.get("apt") or [])
        go_entries = list(chans.get("go") or [])
        npm_entries = list(chans.get("npm") or [])
        cargo_entries = list(chans.get("cargo") or [])
        src_entries = list(chans.get("from-source") or [])

        # --- {domain}: first 75 apt + all go ---
        apt_main = apt_entries[:APT_MAX]
        apt_rest = apt_entries[APT_MAX:]
        main_id = domain
        ensure_comp(main_id)

        for entry in apt_main:
            pkgs = parse_apt_packages(entry["channel_body"])
            # fall back to bin/id as package name
            if not pkgs:
                pkgs = [entry["bin"]] if entry["bin"] else []
            # one bake line per package; tool once in pack
            bake_val = pkgs[0] if pkgs else entry["bin"]
            add_tool_to_comp(main_id, entry, "apt", bake_val)
            for extra in pkgs[1:]:
                buckets[main_id]["apt"].append(extra)

        for entry in go_entries:
            spec = go_install_spec(entry["channel_body"], entry["primary"])
            add_tool_to_comp(main_id, entry, "go", spec or entry["bin"])

        # --- {domain}-2: overflow apt ---
        if apt_rest:
            two_id = f"{domain}-2"
            ensure_comp(two_id)
            for entry in apt_rest:
                pkgs = parse_apt_packages(entry["channel_body"])
                if not pkgs:
                    pkgs = [entry["bin"]] if entry["bin"] else []
                bake_val = pkgs[0] if pkgs else entry["bin"]
                add_tool_to_comp(two_id, entry, "apt", bake_val)
                for extra in pkgs[1:]:
                    buckets[two_id]["apt"].append(extra)

        # --- {domain}-ext: npm + cargo ---
        if npm_entries or cargo_entries:
            ext_id = f"{domain}-ext"
            ensure_comp(ext_id)
            for entry in npm_entries:
                pkg = npm_package(entry["channel_body"], entry["primary"])
                add_tool_to_comp(ext_id, entry, "npm", pkg or entry["bin"])
            for entry in cargo_entries:
                pkg = cargo_package(entry["channel_body"], entry["primary"])
                add_tool_to_comp(ext_id, entry, "cargo", pkg or entry["bin"])

        # --- {domain}-src: first 40 from-source ---
        src_allow = src_entries[:SRC_MAX]
        if src_allow:
            src_id = f"{domain}-src"
            ensure_comp(src_id)
            for entry in src_allow:
                spec = src_spec(entry["channel_body"], entry["tool"])
                add_tool_to_comp(src_id, entry, "from-source", spec or entry["bin"])

    # minimal: not a bulk catalog shard. Pin the handler binaries that no domain
    # image bakes, so a regen cannot wipe the guest back to "daemon only".
    # requests / pwntools / lief stay in the Dockerfile base (every composition).
    # playwright is intentionally absent — sec_browser is not a guest tool.
    ensure_comp("minimal")
    buckets["minimal"]["apt"] = [
        "file",
        "hashcat",
        "wabt",
        "ruby",
        "nodejs",
        "npm",
        "unzip",
        "openjdk-17-jre-headless",
        "libfplll-dev",
        "libgmp-dev",
        "libmpfr-dev",
    ]
    buckets["minimal"]["npm"] = ["webcrack"]
    buckets["minimal"]["src"] = ["https://github.com/RsaCtfTool/RsaCtfTool.git"]
    buckets["minimal"]["pip"] = [
        "z3-solver",
        "ROPgadget",
        "hashid",
        "name-that-hash",
        "checksec.py",
        "jsbeautifier",
        "cryptography",
        "fpylll",
    ]
    buckets["minimal"]["gem"] = ["one_gadget"]
    buckets["minimal"]["bins"] = [
        "file",
        "hashcat",
        "wasm-decompile",
        "wasm2wat",
        "ROPgadget",
        "hashid",
        "nth",
        "checksec",
        "one_gadget",
        "RsaCtfTool",
        "zap.sh",
        "webcrack",
    ]

    # compositions metadata (stable order: domains then shards, minimal last)
    def comp_sort_key(cid: str) -> tuple:
        if cid == "minimal":
            return (1, 99, cid)
        base = cid.split("-")[0]
        try:
            di = DOMAIN_ORDER.index(base)
        except ValueError:
            di = 50
        suffix = cid[len(base) :]  # "", "-2", "-ext", "-src"
        order = {"": 0, "-2": 1, "-ext": 2, "-src": 3}.get(suffix, 9)
        return (0, di, order, cid)

    for comp_id in sorted(buckets.keys(), key=comp_sort_key):
        b = buckets[comp_id]
        apt_n = len({x.lower() for x in b["apt"]})
        channels_present = []
        for ch, key in (
            ("apt", "apt"),
            ("go", "go"),
            ("npm", "npm"),
            ("cargo", "cargo"),
            ("from-source", "src"),
            ("pip", "pip"),
            ("gem", "gem"),
        ):
            if b.get(key):
                channels_present.append(ch)
        domain = comp_id.split("-")[0] if comp_id != "minimal" else "minimal"
        if domain not in DOMAIN_ORDER and domain != "minimal":
            domain = "lab"
        blurb = DOMAIN_BLURB.get(domain, domain)
        if comp_id.endswith("-2"):
            blurb = f"{blurb} (apt overflow shard)"
        elif comp_id.endswith("-ext"):
            blurb = f"{blurb} (npm/cargo)"
        elif comp_id.endswith("-src"):
            blurb = f"{blurb} (from-source allowlist ≤{SRC_MAX})"
        elif comp_id == "minimal":
            blurb = DOMAIN_BLURB["minimal"]
        base_hint = "kali" if apt_n > 25 else "debian"
        compositions.append(
            {
                "id": comp_id,
                "domain": domain if comp_id != "minimal" else "minimal",
                "channels": channels_present,
                "blurb": blurb,
                "base_hint": base_hint,
            }
        )

    # write bucket files
    BUCKETS.mkdir(parents=True, exist_ok=True)
    for comp_id, b in buckets.items():
        write_lines(BUCKETS / f"{comp_id}.apt.txt", b["apt"])
        write_lines(BUCKETS / f"{comp_id}.go.txt", b["go"])
        write_lines(BUCKETS / f"{comp_id}.npm.txt", b["npm"])
        write_lines(BUCKETS / f"{comp_id}.cargo.txt", b["cargo"])
        write_lines(BUCKETS / f"{comp_id}.src.txt", b["src"])
        write_lines(BUCKETS / f"{comp_id}.bins.txt", b["bins"])
        for extra in ("pip", "gem"):
            if b.get(extra):
                write_lines(BUCKETS / f"{comp_id}.{extra}.txt", b[extra])
    (BUCKETS / "minimal.zap.url").write_text(
        "https://github.com/zaproxy/zaproxy/releases/download/v2.17.0/ZAP_2.17.0_Crossplatform.zip\n",
        encoding="utf-8",
    )

    COMPOSITIONS_OUT.write_text(
        json.dumps(compositions, indent=2) + "\n", encoding="utf-8"
    )

    # packs for domain compositions (not minimal) — leave smoke.json untouched
    PACKS.mkdir(parents=True, exist_ok=True)
    packs_written: list[str] = []
    for comp_id, tools in pack_tools.items():
        if comp_id == "minimal":
            continue
        out = {"version": 1, "pack": comp_id, "tools": tools}
        (PACKS / f"{comp_id}.json").write_text(
            json.dumps(out, indent=2) + "\n", encoding="utf-8"
        )
        packs_written.append(comp_id)

    # summary
    def counts_for(comp_id: str) -> dict:
        b = buckets[comp_id]
        return {
            "apt": len({x.lower() for x in b["apt"]}),
            "go": len(b["go"]),
            "npm": len(b["npm"]),
            "cargo": len(b["cargo"]),
            "src": len(b["src"]),
            "bins": len({x for x in b["bins"]}),
            "tools_in_pack": len(pack_tools.get(comp_id) or []),
        }

    summary = {
        "tools_in": len(tools_in),
        "skipped_snap": skipped_snap,
        "skipped_other": skipped_other,
        "domain_tool_counts": dict(sorted(domain_counts.items())),
        "category_counts": dict(sorted(category_counts.items(), key=lambda kv: (-kv[1], kv[0]))),
        "compositions": {
            c["id"]: {**counts_for(c["id"]), "base_hint": c["base_hint"], "channels": c["channels"]}
            for c in compositions
        },
        "composition_ids": [c["id"] for c in compositions],
        "packs_written": sorted(packs_written),
        "rules": {
            "apt_max_per_domain_image": APT_MAX,
            "src_allowlist_max": SRC_MAX,
            "snap": "skipped",
            "channel": "preferred_order[0]; source→from-source",
        },
    }
    SUMMARY_OUT.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    # COMPOSITIONS.md
    lines = [
        "# koma-safety domain compositions",
        "",
        "Generated by `gen_domain_buckets.py`. Bake lists live under `buckets/`.",
        "",
        "| id | domain | channels | base_hint | apt | go | npm | cargo | src | tools |",
        "|---|---|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for c in compositions:
        ct = counts_for(c["id"])
        chs = ",".join(c["channels"]) if c["channels"] else "—"
        lines.append(
            f"| `{c['id']}` | {c['domain']} | {chs} | {c['base_hint']} | "
            f"{ct['apt']} | {ct['go']} | {ct['npm']} | {ct['cargo']} | {ct['src']} | "
            f"{ct['tools_in_pack']} |"
        )
    lines.extend(
        [
            "",
            "Shard rules: `{domain}` = apt(≤75)+go; `{domain}-2` = apt overflow; "
            "`{domain}-ext` = npm+cargo; `{domain}-src` = from-source allowlist ≤40; "
            "`minimal` = handler binaries missing from every domain image. Snap skipped.",
            "",
        ]
    )
    COMPOSITIONS_MD.write_text("\n".join(lines), encoding="utf-8")

    # stdout report
    report = {
        "composition_ids": [c["id"] for c in compositions],
        "tool_counts": {c["id"]: counts_for(c["id"])["tools_in_pack"] for c in compositions},
        "channel_counts": {
            c["id"]: {
                k: counts_for(c["id"])[k]
                for k in ("apt", "go", "npm", "cargo", "src", "bins")
            }
            for c in compositions
        },
        "skipped_snap": skipped_snap,
        "packs": sorted(packs_written),
    }
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())

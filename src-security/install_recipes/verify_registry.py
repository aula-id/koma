#!/usr/bin/env python3
"""Check every npm / cargo package the catalog names against its registry.

The source list guesses that a tool is published under its own name. Most are
not: the name belongs to an unrelated package, or to nothing. Installing by
name alone bakes a stranger's package into a guest image and lists it as a
security tool.

This writes `registry_audit.json`, which `gen_domain_buckets.py` reads. A
package is installed from a registry only when it is `verified`:

  repo         the registry entry points at the tool's own repository
  description  the tool has no repository on record, and the registry entry
               carries the same description (and, on npm, ships the command)

Anything else is left out of the registry lists. Run this, then the generator:

  python3 verify_registry.py && python3 gen_domain_buckets.py
"""
from __future__ import annotations

import json
import re
import sys
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SRC = ROOT / "pentest_tools_cli_debian_linux.json"
OUT = ROOT / "registry_audit.json"
AGENT = "catalog-registry-audit"
MIN_DESCRIPTION = 20


def repo_key(url: object) -> str:
    """`owner/name`, lowercased, whatever the URL was dressed in."""
    if isinstance(url, dict):
        url = url.get("url")
    s = str(url or "").strip().lower()
    s = re.sub(r"^git\+", "", s)
    s = re.sub(r"^(git|https?|ssh)://", "", s)
    s = re.sub(r"^git@", "", s)
    s = s.replace("github.com:", "github.com/")
    s = re.sub(r"^(www\.)?github\.com/", "", s)
    s = re.sub(r"\.git$", "", s.split("#")[0].rstrip("/"))
    return s


def text_key(text: object) -> str:
    return re.sub(r"[^a-z0-9]+", " ", str(text or "").lower()).strip()


def fetch(url: str) -> dict | None:
    """The registry document, or None when the package does not exist."""
    req = urllib.request.Request(url, headers={"User-Agent": AGENT})
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.load(resp)
    except urllib.error.HTTPError as err:
        if err.code == 404:
            return None
        raise


def judge(tool: dict, repo: str, description: str, has_command: bool) -> tuple[bool, str]:
    want = repo_key(tool.get("repository"))
    if want:
        if repo and repo == want:
            return True, "repo"
        return False, "another repository" if repo else "no repository on the registry entry"
    ours = text_key(tool.get("description"))
    if len(ours) >= MIN_DESCRIPTION and ours == text_key(description):
        if has_command:
            return True, "description"
        return False, "same description but ships no command"
    return False, "tool has no repository on record and the description differs"


def check_npm(package: str, tool: dict) -> dict:
    doc = fetch(f"https://registry.npmjs.org/{package}/latest")
    if doc is None:
        return {"verified": False, "why": "not on the registry"}
    bins = doc.get("bin")
    commands = sorted(bins) if isinstance(bins, dict) else ([package] if bins else [])
    repo = repo_key(doc.get("repository"))
    ok, why = judge(tool, repo, doc.get("description") or "", bool(commands))
    if ok and not commands:
        ok, why = False, "ships no command"
    return {"verified": ok, "why": why, "repository": repo, "commands": commands}


def check_cargo(package: str, tool: dict) -> dict:
    doc = fetch(f"https://crates.io/api/v1/crates/{package}")
    if doc is None:
        return {"verified": False, "why": "not on the registry"}
    crate = doc.get("crate") or {}
    repo = repo_key(crate.get("repository"))
    ok, why = judge(tool, repo, crate.get("description") or "", True)
    return {"verified": ok, "why": why, "repository": repo}


def wanted() -> dict[str, dict[str, dict]]:
    """channel -> package -> tool, for every tool whose first choice is a registry."""
    from gen_domain_buckets import cargo_package, npm_package, preferred_channel

    out: dict[str, dict[str, dict]] = {"npm": {}, "cargo": {}}
    for tool in json.loads(SRC.read_text(encoding="utf-8")).get("tools") or []:
        install = tool.get("install") if isinstance(tool.get("install"), dict) else {}
        ch = preferred_channel(install)
        if ch not in out:
            continue
        body = (install.get("channels") or {}).get(ch) or {}
        primary = str(install.get("primary_command") or "")
        pick = npm_package if ch == "npm" else cargo_package
        package = pick(body, primary) or str(tool.get("bin") or tool.get("id") or "")
        if package:
            out[ch].setdefault(package, tool)
    return out


def main() -> int:
    audit: dict[str, dict[str, dict]] = {}
    for channel, check, workers in (("npm", check_npm, 8), ("cargo", check_cargo, 2)):
        packages = sorted(wanted()[channel].items())
        with ThreadPoolExecutor(workers) as pool:
            results = list(pool.map(lambda kv: check(kv[0], kv[1]), packages))
        audit[channel] = {name: res for (name, _), res in zip(packages, results)}
        good = sum(1 for r in results if r["verified"])
        print(f"{channel}: {good} verified of {len(results)}", file=sys.stderr)
    OUT.write_text(json.dumps(audit, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())

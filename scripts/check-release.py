#!/usr/bin/env python3
"""Check a published release.json against the tag and commit that built it.

Usage: check-release.py FILE VERSION SHA
"""

import json
import re
import sys

SHA = re.compile(r"^[0-9a-f]{40}$")


def check(document: dict, version: str, sha: str) -> None:
    if document.get("version") != version:
        raise SystemExit(f"release version is {document.get('version')}, expected {version}")
    got = str(document.get("sha", "")).lower()
    if not SHA.fullmatch(got) or got != sha.lower():
        raise SystemExit(f"release sha is {document.get('sha')}, expected {sha}")


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit("usage: check-release.py FILE VERSION SHA")
    with open(sys.argv[1], encoding="utf-8") as stream:
        document = json.load(stream)
    if not isinstance(document, dict):
        raise SystemExit("release.json must be an object")
    check(document, sys.argv[2], sys.argv[3])


if __name__ == "__main__":
    main()

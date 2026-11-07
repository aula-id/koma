#!/usr/bin/env python3
"""Write a release tag into the crate version. Does not commit.

Usage: stamp-release.py VERSION [ROOT]
VERSION is MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-preview.2.
"""

import re
import sys
from pathlib import Path

VERSION = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
CARGO_VERSION = re.compile(r'^(version = ")[^"]+(")\s*$', re.MULTILINE)
LOCK_VERSION = re.compile(r'(\[\[package\]\]\nname = "agent"\nversion = ")[^"]+(")')


def stamp(root: Path, version: str) -> None:
    if VERSION.fullmatch(version) is None:
        raise SystemExit(f"stamp-release: not a semver version: {version}")
    cargo = root / "src-agent" / "Cargo.toml"
    cargo_text = cargo.read_text(encoding="utf-8")
    updated, count = CARGO_VERSION.subn(rf"\g<1>{version}\2", cargo_text, count=1)
    if count != 1:
        raise SystemExit("stamp-release: Cargo.toml version line not found")
    cargo.write_text(updated, encoding="utf-8")

    lock = root / "Cargo.lock"
    lock_text = lock.read_text(encoding="utf-8")
    updated, count = LOCK_VERSION.subn(rf"\g<1>{version}\2", lock_text, count=1)
    if count != 1:
        raise SystemExit("stamp-release: Cargo.lock agent version not found")
    lock.write_text(updated, encoding="utf-8")


def main() -> None:
    if len(sys.argv) not in (2, 3):
        raise SystemExit("usage: stamp-release.py VERSION [ROOT]")
    root = Path(sys.argv[2]) if len(sys.argv) == 3 else Path(__file__).resolve().parents[1]
    stamp(root, sys.argv[1])


if __name__ == "__main__":
    main()

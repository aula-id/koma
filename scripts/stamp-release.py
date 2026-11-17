#!/usr/bin/env python3
"""Write a release tag into the crate version. Does not commit.

Usage: stamp-release.py [--msi] VERSION [ROOT]
VERSION is MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-preview.2.

--msi also sets the packaging version that cargo-packager gives the Windows
installer to the numeric MAJOR.MINOR.PATCH only. WiX accepts digits only, so a
tag such as 0.4.0-preview.2 would otherwise fail the MSI build. koma.exe itself
keeps the full version.
"""

import re
import sys
from pathlib import Path

VERSION = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
CARGO_VERSION = re.compile(r'^(version = ")[^"]+(")\s*$', re.MULTILINE)
LOCK_VERSION = re.compile(r'(\[\[package\]\]\nname = "agent"\nversion = ")[^"]+(")')
PACKAGER_HEADER = "[package.metadata.packager]"


def msi_version(version: str) -> str:
    """The numeric core of a release version, within WiX's limits."""
    match = VERSION.fullmatch(version)
    if match is None:
        raise SystemExit(f"stamp-release: not a semver version: {version}")
    major, minor, patch = (int(part) for part in match.groups())
    if major > 255 or minor > 255 or patch > 65535:
        raise SystemExit(f"stamp-release: {version} is outside the MSI version limits")
    return f"{major}.{minor}.{patch}"


def set_packager_version(cargo_text: str, version: str) -> str:
    """Set `version` inside [package.metadata.packager], adding it if absent."""
    lines = cargo_text.split("\n")
    try:
        start = lines.index(PACKAGER_HEADER)
    except ValueError:
        raise SystemExit(f"stamp-release: {PACKAGER_HEADER} not found in Cargo.toml")
    end = start + 1
    while end < len(lines) and not lines[end].startswith("["):
        end += 1
    entry = f'version = "{version}"'
    for i in range(start + 1, end):
        if re.match(r"^version\s*=", lines[i]):
            lines[i] = entry
            return "\n".join(lines)
    lines.insert(start + 1, entry)
    return "\n".join(lines)


def stamp(root: Path, version: str, msi: bool = False) -> None:
    if VERSION.fullmatch(version) is None:
        raise SystemExit(f"stamp-release: not a semver version: {version}")
    cargo = root / "src-agent" / "Cargo.toml"
    cargo_text = cargo.read_text(encoding="utf-8")
    updated, count = CARGO_VERSION.subn(rf"\g<1>{version}\2", cargo_text, count=1)
    if count != 1:
        raise SystemExit("stamp-release: Cargo.toml version line not found")
    if msi:
        updated = set_packager_version(updated, msi_version(version))
    cargo.write_text(updated, encoding="utf-8")

    lock = root / "Cargo.lock"
    lock_text = lock.read_text(encoding="utf-8")
    updated, count = LOCK_VERSION.subn(rf"\g<1>{version}\2", lock_text, count=1)
    if count != 1:
        raise SystemExit("stamp-release: Cargo.lock agent version not found")
    lock.write_text(updated, encoding="utf-8")


def main() -> None:
    args = sys.argv[1:]
    msi = "--msi" in args
    args = [a for a in args if a != "--msi"]
    if len(args) not in (1, 2):
        raise SystemExit("usage: stamp-release.py [--msi] VERSION [ROOT]")
    root = Path(args[1]) if len(args) == 2 else Path(__file__).resolve().parents[1]
    stamp(root, args[0], msi)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Fail a Linux release if its AppImage cannot load its bundled English OCR.

Runs after cargo-packager, against the artifact that will be uploaded. This
checks the actual executable, libraries and language data, not host Tesseract.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def check(directory: Path) -> None:
    images = sorted(directory.glob("koma*.AppImage"))
    if not images:
        raise RuntimeError(f"No AppImage in {directory}")
    for artifact in images:
        with tempfile.TemporaryDirectory(prefix="koma-package-check-") as scratch:
            subprocess.run([str(artifact.resolve()), "--appimage-extract"], cwd=scratch,
                           check=True, stdout=subprocess.DEVNULL, timeout=120)
            app = Path(scratch) / "squashfs-root"
            binary = app / "usr/bin/tesseract"
            languages = list((app / "usr/share/tesseract-ocr").glob("*/tessdata/eng.traineddata"))
            if not binary.is_file() or len(languages) != 1:
                raise RuntimeError(f"{artifact}: missing executable or ambiguous/missing English data")
            environment = os.environ.copy()
            environment["TESSDATA_PREFIX"] = str(languages[0].parent)
            environment["LD_LIBRARY_PATH"] = os.pathsep.join(str(p) for p in (app / "usr/lib", app / "usr/lib64"))
            result = subprocess.run([str(binary), "--list-langs"], env=environment,
                                    check=True, capture_output=True, text=True, timeout=15)
            if "eng" not in result.stdout.splitlines():
                raise RuntimeError(f"{artifact}: bundled Tesseract did not load English")
            # Listing languages only checks filenames. An actual OCR invocation
            # detects broken runtime libraries or incompatible traineddata.
            fixture = Path(scratch) / "blank.pgm"
            fixture.write_bytes(b"P5\n100 50\n255\n" + b"\xff" * 5000)
            subprocess.run([str(binary), str(fixture), "stdout", "-l", "eng", "--psm", "6"],
                           env=environment, check=True, capture_output=True, timeout=15)
            if not list(app.rglob("libXtst.so*")):
                raise RuntimeError(f"{artifact}: missing XTEST library")
            print(f"{artifact.name}: bundled English OCR and XTEST present")


if __name__ == "__main__":
    check(Path(sys.argv[1] if len(sys.argv) > 1 else "target/release"))

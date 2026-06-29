"""
install_manifest — single source of truth for every external dependency.

MANIFEST is a flat list of dependency descriptor dicts, one per external dep,
derived from TOOLKIT.md. health.py reads it to report presence; installer.py
reads it to install Tier-1 (pip) and Tier-2 (binary/gem) deps. Tier-3 deps are
detect-only — install returns their manual hint.

Stdlib only at module level (os, platform) — this module is imported during
daemon startup with ZERO toolkit installed, so it MUST NOT pull in requests,
etc. (those are imported lazily inside installer.py).

Descriptor schema
-----------------
  key          unique id, e.g. "nuclei"
  name         display name
  tier         1 | 2 | 3
  method       "pip" | "binary" | "gem" | "manual"
  detect       presence probe argument
  detect_kind  "which" (shutil.which) | "import" (importlib.util.find_spec)
  tools        sec_ tool names that need this dep
  hint         human install command (always present; the ONLY field returned
               for tier-3 manual deps)
  pip          (method=="pip")     pip package name(s)
  post         (method=="pip", opt) extra argv lists run with sys.executable
  repo         (method=="binary")  GitHub "owner/name"
  asset_re     (method=="binary")  regex matching the linux release asset name
  members      (method=="binary")  files to extract from the archive into bin/
  gem          (method=="gem")     gem name
"""

import os
import platform


def arch_tag() -> str:
    """
    Map the host CPU to the release-asset arch token used in download names.

    x86_64            -> "amd64"
    aarch64 / arm64   -> "arm64"
    anything else     -> the raw platform.machine() value (best-effort; the
                         asset regex simply won't match and the installer
                         returns a clear "no matching asset" error).
    """
    m = platform.machine().lower()
    if m in ("x86_64", "amd64", "x64"):
        return "amd64"
    if m in ("aarch64", "arm64"):
        return "arm64"
    return m


def bin_dir() -> str:
    """Absolute path to the persisted binary dir: ~/.koma/security/bin."""
    return os.path.expanduser("~/.koma/security/bin")


_ARCH = arch_tag()

MANIFEST: list[dict] = [
    # ----------------------------------------------------------------- TIER 1
    # Python libraries (detect via import) + pip-installed CLIs (detect via which).
    {
        "key": "requests",
        "name": "requests",
        "tier": 1,
        "method": "pip",
        "detect": "requests",
        "detect_kind": "import",
        "tools": ["sec_http", "sec_zap", "sec_factor"],
        "hint": "pip install requests",
        "pip": ["requests>=2.31"],
    },
    {
        "key": "pwntools",
        "name": "pwntools",
        "tier": 1,
        "method": "pip",
        "detect": "pwn",
        "detect_kind": "import",
        "tools": ["sec_remote", "sec_pwntmpl", "sec_triage", "sec_rop"],
        "hint": "pip install pwntools>=4.15",
        # unicorn pinned to 2.1.1: pwntools 4.15 excludes 2.1.3/2.1.4 (the versions
        # that ship a macOS arm64 wheel) and 2.1.2 is source-only, forcing a QEMU
        # compile that fails on Apple Clang 17 (Int128 typedef redefinition). 2.1.1
        # ships a prebuilt arm64 wheel and is pwntools-compatible — no compiler needed.
        "pip": ["unicorn==2.1.1", "pwntools>=4.15"],
    },
    {
        "key": "z3-solver",
        "name": "z3-solver",
        "tier": 1,
        "method": "pip",
        "detect": "z3",
        "detect_kind": "import",
        "tools": ["sec_z3"],
        "hint": "pip install z3-solver",
        "pip": ["z3-solver"],
    },
    {
        "key": "fpylll",
        "name": "fpylll",
        "tier": 1,
        "method": "pip",
        "detect": "fpylll",
        "detect_kind": "import",
        "tools": ["sec_lattice"],
        "hint": "pip install fpylll  (may require libfplll-dev on Debian/Ubuntu)",
        "pip": ["fpylll"],
    },
    {
        "key": "jsbeautifier",
        "name": "jsbeautifier",
        "tier": 1,
        "method": "pip",
        "detect": "jsbeautifier",
        "detect_kind": "import",
        "tools": ["sec_unmin", "sec_jsdeobf", "sec_sourcemap"],
        "hint": "pip install jsbeautifier",
        "pip": ["jsbeautifier"],
    },
    {
        "key": "playwright",
        "name": "playwright (+ chromium)",
        "tier": 1,
        "method": "pip",
        "detect": "playwright",
        "detect_kind": "import",
        "tools": ["sec_xss_confirm"],
        "hint": "pip install playwright && playwright install chromium",
        "pip": ["playwright"],
        "post": [["-m", "playwright", "install", "chromium"]],
    },
    {
        "key": "sqlmap",
        "name": "sqlmap",
        "tier": 1,
        "method": "pip",
        "detect": "sqlmap",
        "detect_kind": "which",
        "tools": ["sec_sqlmap"],
        "hint": "pip install sqlmap  or  apt install sqlmap",
        "pip": ["sqlmap"],
    },
    {
        "key": "RsaCtfTool",
        "name": "RsaCtfTool",
        "tier": 3,
        "method": "manual",
        "detect": "RsaCtfTool",
        "detect_kind": "which",
        "tools": ["sec_rsa"],
        "hint": "git clone https://github.com/RsaCtfTool/RsaCtfTool && pip install -r RsaCtfTool/requirements.txt  (needs Python <=3.11; not pip-installable on 3.12)",
    },
    {
        "key": "ROPgadget",
        "name": "ROPgadget",
        "tier": 1,
        "method": "pip",
        "detect": "ROPgadget",
        "detect_kind": "which",
        "tools": ["sec_rop"],
        "hint": "pip install ROPgadget",
        "pip": ["ROPgadget"],
    },
    {
        "key": "name-that-hash",
        "name": "name-that-hash",
        "tier": 1,
        "method": "pip",
        "detect": "nth",
        "detect_kind": "which",
        "tools": ["sec_hashid"],
        "hint": "pip install name-that-hash",
        "pip": ["name-that-hash"],
    },
    {
        "key": "hashid",
        "name": "hashid",
        "tier": 1,
        "method": "pip",
        "detect": "hashid",
        "detect_kind": "which",
        "tools": ["sec_hashid"],
        "hint": "pip install hashid",
        "pip": ["hashid"],
    },
    {
        "key": "checksec",
        "name": "checksec",
        "tier": 1,
        "method": "pip",
        "detect": "checksec",
        "detect_kind": "which",
        "tools": ["sec_triage"],
        "hint": "pip install checksec.py  or  apt install checksec",
        "pip": ["checksec.py"],
    },
    # ----------------------------------------------------------------- TIER 2
    # Downloadable single-file binaries from GitHub releases.
    {
        "key": "nuclei",
        "name": "nuclei",
        "tier": 2,
        "method": "binary",
        "detect": "nuclei",
        "detect_kind": "which",
        "tools": ["sec_nuclei"],
        "hint": "go install github.com/projectdiscovery/nuclei/v3/cmd/nuclei@latest"
                "  or  download from https://github.com/projectdiscovery/nuclei/releases",
        "repo": "projectdiscovery/nuclei",
        "asset_re": rf"nuclei_.*linux_{_ARCH}\.zip$",
        "members": ["nuclei"],
    },
    {
        "key": "ffuf",
        "name": "ffuf",
        "tier": 2,
        "method": "binary",
        "detect": "ffuf",
        "detect_kind": "which",
        "tools": ["sec_ffuf"],
        "hint": "go install github.com/ffuf/ffuf/v2@latest"
                "  or  download from https://github.com/ffuf/ffuf/releases",
        "repo": "ffuf/ffuf",
        "asset_re": rf"ffuf_.*linux_{_ARCH}\.tar\.gz$",
        "members": ["ffuf"],
    },
    {
        "key": "dalfox",
        "name": "dalfox",
        "tier": 2,
        "method": "binary",
        "detect": "dalfox",
        "detect_kind": "which",
        "tools": ["sec_dalfox"],
        "hint": "go install github.com/hahwul/dalfox/v2@latest"
                "  or  download from https://github.com/hahwul/dalfox/releases",
        "repo": "hahwul/dalfox",
        "asset_re": rf"dalfox_.*linux_{_ARCH}\.tar\.gz$",
        "members": ["dalfox"],
    },
    {
        "key": "wabt",
        "name": "wabt (wasm-decompile, wasm2wat)",
        "tier": 2,
        "method": "binary",
        "detect": "wasm-decompile",
        "detect_kind": "which",
        "tools": ["sec_wasm"],
        "hint": "apt install wabt  or  download from https://github.com/WebAssembly/wabt/releases",
        "repo": "WebAssembly/wabt",
        # wabt release assets are named like wabt-1.0.36-ubuntu-20.04.tar.gz /
        # wabt-1.0.36-linux.tar.gz — match either "linux" or "ubuntu".
        "asset_re": r"wabt-.*(linux|ubuntu).*\.tar\.gz$",
        "members": ["wasm-decompile", "wasm2wat"],
    },
    # ----------------------------------------------------------------- TIER 2 (gem)
    {
        "key": "one_gadget",
        "name": "one_gadget",
        "tier": 2,
        "method": "gem",
        "detect": "one_gadget",
        "detect_kind": "which",
        "tools": ["sec_rop"],
        "hint": "gem install one_gadget  (requires Ruby >= 2.6)",
        "gem": "one_gadget",
    },
    # ----------------------------------------------------------------- TIER 3
    # Heavy / system deps — detect only. install returns the manual hint.
    {
        "key": "sage",
        "name": "SageMath",
        "tier": 3,
        "method": "manual",
        "detect": "sage",
        "detect_kind": "which",
        "tools": ["sec_sage"],
        "hint": "apt install sagemath  or  https://www.sagemath.org/",
    },
    {
        "key": "hashcat",
        "name": "hashcat",
        "tier": 3,
        "method": "manual",
        "detect": "hashcat",
        "detect_kind": "which",
        "tools": ["sec_crack"],
        "hint": "apt install hashcat  + a compatible GPU driver  (or https://hashcat.net/hashcat/)",
    },
    {
        "key": "node",
        "name": "Node.js / npx",
        "tier": 3,
        "method": "manual",
        "detect": "node",
        "detect_kind": "which",
        "tools": ["sec_jsdeobf", "sec_unmin"],
        "hint": "apt install nodejs npm  (needed for `npx webcrack`)",
    },
    {
        "key": "zap",
        "name": "OWASP ZAP",
        "tier": 3,
        "method": "manual",
        "detect": "zap.sh",
        "detect_kind": "which",
        "tools": ["sec_zap"],
        "hint": "download from https://www.zaproxy.org/ , then run "
                "`zap.sh -daemon -port 8080 -config api.key=<key>`",
    },
]

# Fast lookup by key for the installer.
KEY_INDEX: dict[str, dict] = {d["key"]: d for d in MANIFEST}

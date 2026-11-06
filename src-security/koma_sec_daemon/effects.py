"""Effect classification: how far each operation's side effects reach.

Labels are for routing and enforcement. They are NOT a consent system — security
mode deliberately has no per-action prompts, and turning one into a "risk tier"
would reintroduce them by accident.

Two labels with a fixed meaning instead:

EFFECT  (SecRisk, mirrored in Rust)
  static   pure computation; no target, no filesystem read of the user's tree
  local    reads a local path or the local binary's own resources
  remote   sends packets to a target in scope
  exploit  sends a crafted payload; effects go beyond observation

EXEC (what the operation needs to run at all)
  cpu      nothing but an interpreter
  binary   an external binary from the install manifest
  net      the enforced egress path
  browser  the contained browser and its authenticated control channel

`remote`/`exploit` are NOT implied by `net`: `sec_zap` and `sec_ffuf` sweep a
target, `sec_sqlmap` and `sec_dalfox` fire payloads. `http` and `xss_confirm` are
`net` but classified by effect, never as static just because a single request
looks small.

Argument-dependent network access is why classification cannot be inferred from
the schema alone: `sec_remote` takes `host` and only reaches the network when it
is set. A tool whose effect depends on its arguments declares the deciding
parameter in `depends_on`; a call without it is admitted at the lower effect and
re-classified at dispatch.
"""

from __future__ import annotations

STATIC = "static"
LOCAL = "local"
REMOTE = "remote"
EXPLOIT = "exploit"
EFFECTS = (STATIC, LOCAL, REMOTE, EXPLOIT)

CPU = "cpu"
BINARY = "binary"
NET = "net"
BROWSER = "browser"
EXECS = (CPU, BINARY, NET, BROWSER)

# name -> (effect, exec, depends_on, note)
#
# `depends_on` names the argument that decides the effect when it is weaker
# without it. Provisional until phase 5 restores the handlers; the validator
# rejects a row whose declared effect the schema cannot support, so a wrong guess
# surfaces then rather than as a silently mis-routed call.
MANIFEST: dict[str, tuple[str, str, str | None, str]] = {
    "sec_http": (REMOTE, NET, None, "one request to one URL"),
    "sec_remote": (REMOTE, NET, "host", "local only when run without a host"),
    "sec_sqlmap": (EXPLOIT, NET, None, "fires injection payloads"),
    "sec_nuclei": (REMOTE, NET, None, "template sweep"),
    "sec_ffuf": (REMOTE, NET, None, "content/param fuzzing"),
    "sec_dalfox": (EXPLOIT, NET, None, "XSS payload firing"),
    "sec_zap": (REMOTE, BROWSER, None, "active scan via the browser channel"),
    "sec_xss_confirm": (EXPLOIT, BROWSER, None, "stored/reflected payload in a real browser"),
    "sec_triage": (LOCAL, CPU, None, "reads an artifact from disk"),
    "sec_sourcemap": (LOCAL, CPU, None, "reads a local map file"),
    "sec_wasm": (LOCAL, BINARY, None, "reads a local module"),
    "sec_rop": (LOCAL, BINARY, None, "reads a target binary"),
    "sec_crack": (LOCAL, BINARY, None, "reads the wordlist"),
    "sec_z3": (STATIC, CPU, None, "SMT solve"),
    "sec_sage": (STATIC, BINARY, None, "SageMath solve"),
    "sec_rsa": (STATIC, CPU, None, "RSA arithmetic"),
    "sec_factor": (STATIC, BINARY, None, "factorisation"),
    "sec_lattice": (STATIC, BINARY, None, "lattice reduction"),
    "sec_hashid": (STATIC, CPU, None, "hash identification"),
    "sec_decode": (STATIC, CPU, None, "encoding transforms"),
    "sec_jsdeobf": (STATIC, BINARY, None, "deobfuscation"),
    "sec_unmin": (STATIC, BINARY, None, "unminify"),
    "sec_pwntmpl": (STATIC, CPU, None, "emit a pwntools template"),
}

# Harness-level caps. Argument names read by the egress gate, weakest first.
DESTINATION_PARAMS = ("url", "host", "target", "hostname", "endpoint")


def effect_of(name: str, args: dict | None = None) -> str | None:
    """
    The effect to apply for THIS call, or None for an unknown tool.

    An argument-dependent row drops to `local` when its deciding parameter is
    absent, so `sec_remote {}` is not admitted as network-capable.
    """
    row = MANIFEST.get(name)
    if row is None:
        return None
    effect, _exec, depends_on, _note = row
    if depends_on and not (args or {}).get(depends_on):
        return LOCAL
    return effect


def destination_of(args: dict | None) -> str | None:
    """The destination argument a call carries, if any."""
    for key in DESTINATION_PARAMS:
        value = (args or {}).get(key)
        if isinstance(value, str) and value.strip():
            return value
    return None

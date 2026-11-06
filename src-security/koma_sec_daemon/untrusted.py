"""Trust framing for untrusted text.

Everything a target controls — response bodies, corpus text, tool output read back
from a target — is data, never instruction. Tags alone do not achieve that: a body
containing `</untrusted>` closes the wrapper and everything after it looks trusted.

The framing is therefore structural, not lexical:

1. The whole body is **newline-padded**, so a literal delimiter on the first or last
   line cannot join the fence.
2. The body is **not escaped**. Escaping would rewrite bytes the model must match
   exactly (a repro payload has to survive intact), so the fence is placed where a
   closing tag is harmless rather than removed.
3. A machine-readable header sits ABOVE the fence, outside the untrusted region, so
   the origin, verdict and handle stay trusted even if the body forges one.
4. On skim, a body that contains a closing delimiter is reported so the model knows
   to distrust any tag-looking line inside it.

`skim` is deliberately crude and cheap. It is there to raise suspicion, not to
parse: taint reasoning that depends on parsing attacker text is another way to be
wrong.
"""

from __future__ import annotations

FENCE_OPEN = "<<<UNTRUSTED"
FENCE_CLOSE = "UNTRUSTED>>>"

# Origins the model reasons about. Kept closed so a caller cannot invent one.
RESPONSE = "response"
CORPUS = "corpus"
TOOL_OUTPUT = "tool-output"
SOURCE = "source"
ORIGINS = (RESPONSE, CORPUS, TOOL_OUTPUT, SOURCE)


def wrap(body: str, origin: str = RESPONSE, hint: str = "") -> str:
    """
    Frame `body` as untrusted data.

    `origin` and `hint` are trusted metadata and land in the header, above the
    fence. `hint` describes the shape ("json array of 12 objects") so the model can
    decide how to read the body without being told by the body.
    """
    if origin not in ORIGINS:
        raise ValueError(f"unknown origin: {origin}")
    header = f"[{origin}]"
    if hint:
        header += f" {hint}"
    return f"{header}\n{FENCE_OPEN}\n{body}\n{FENCE_CLOSE}"


def skim(body: str) -> str:
    """
    One short sentence about a suspect body, or "" when nothing looks odd.

    Only checks for the delimiter and for instruction-looking lines; both are
    cheap and both are the cases that actually bite.
    """
    # Case-insensitive, and both spellings: the fence is a bare token
    # (`UNTRUSTED>>>`), so a body carrying the natural XML form (`</untrusted>`)
    # would not match it and would slip through on the exact case that matters.
    upper = body.upper()
    if any(tok in upper for tok in (FENCE_CLOSE, FENCE_OPEN, "</UNTRUSTED>", "<UNTRUSTED>")):
        notes = ["body contains a fence token"]
    else:
        notes = []
    lowered = body.lower()
    for marker in ("ignore previous", "ignore the above", "system:", "you are now"):
        if marker in lowered:
            notes.append(f"body mentions '{marker}'")
            break
    if not notes:
        return ""
    return "skim: " + "; ".join(notes)


def wrap_with_skim(body: str, origin: str = RESPONSE, hint: str = "") -> str:
    """`wrap` plus a skim note in the trusted header when the body warrants one."""
    note = skim(body)
    if note:
        hint = f"{hint} — {note}" if hint else note
    return wrap(body, origin, hint)

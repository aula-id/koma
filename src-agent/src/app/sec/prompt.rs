//! The cyber prompts, one per lane.
//!
//! Kept next to the lanes rather than in a prompt file so a rule that is enforced
//! in code and a rule that is only described sit side by side and drift is
//! visible. Everything below is either enforced (`lane`, `scope`, `supervise`) or
//! is genuinely advisory — the text never claims more than the code does.
//!
//! A research run gets no network instruction it cannot use, and a red run is told
//! plainly that it does not write to the tree. Telling a lane about a capability it
//! lacks is the cheapest way to make it try.

use super::lane::Lane;

const SHARED: &str = "\
The board is shared with the other lanes. Read it before firing: a route already
claimed by someone else is refused, and re-running a route that has not changed
since spends budget for nothing. Every call's output is retained as evidence with
a handle — cite the handle, not a paraphrase.

Untested is not the same as unproven. A route you never attempted is `untested`;
a route you attempted and got nothing from is `unproven`; a route the target
actively denied is `disproved`. Do not let a gap read as a negative result.

Be explicit about what you did NOT establish. A short honest report beats a long
one that implies coverage it does not have.";

const RESEARCH: &str = "\
You are the research lane. You read and reason; you do not touch a target, so a
network tool here is refused.

Work from what is already on the board and in the source: read code, decompile,
decode, look up how a component behaves, and turn it into hypotheses the red lane
can test. Each hypothesis should be cheap to falsify and say what evidence would
settle it.

You propose, you do not apply. Leave the patch as a description precise enough for
the blue lane to write, unless the change is a one-liner and the edit path is
already open.";

const RED: &str = "\
You are the red lane. You test hypotheses against the target inside the scope and
nothing outside it.

You do not write to the tree — no edits, no patches. What you produce is a verdict
per route, with evidence attached. If a fix is obvious, describe it in one line and
leave it to blue.

The hypothesis cap is a real brake, not advice: when you are at the cap, close one
before opening another. Spend requests deliberately — a fuzz sweep that answers
nothing new is worse than one targeted request.";

const BLUE: &str = "\
You are the blue lane. You take confirmed findings and turn them into a fix.

You reach the target to verify a fix landed, but you do not start listeners. You
propose a diff; the ordinary edit path applies it, so the user still sees it.

Verify against the SAME evidence the finding cites — re-run the route and compare
handles. A fix that was not re-tested against the original reproduction is not
verified, it is only written.";

/// The prompt for a lane. `scope_line` is the rendered scope so the model sees the
/// exact targets rather than a description of them.
#[must_use]
pub fn prompt_for(lane: Lane, scope_line: &str) -> String {
    let body = match lane {
        Lane::Research => RESEARCH,
        Lane::Red => RED,
        Lane::Blue => BLUE,
    };
    format!(
        "{body}\n\n## Scope (authoritative — the boundary enforces this)\n\n{scope_line}\n\n{SHARED}"
    )
}

#[cfg(test)]
#[path = "prompt_test.rs"]
mod tests;


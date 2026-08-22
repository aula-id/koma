//! Smart, semantic filters for well-known command families (cargo, git) that
//! need block-aware parsing rather than a flat regex strip/keep pipeline (see
//! `super::spec`). Tried before the static spec table in `filter_output`.

pub(crate) mod cargo;
pub(crate) mod git;

use super::FilterOutcome;

/// Try each smart-filter domain in turn against `command`. `raw` is the full
/// buffered command output, `exit_code` the process exit status (`None` if
/// unknown). Returns `None` when no smart filter claims the command — callers
/// then fall through to the static spec table / passthrough.
///
/// `cd X && <cmd>` prefixes are handled by matching against the segment after
/// the last `&&` (which is just `command` itself when there's no `&&`).
pub(crate) fn try_smart(command: &str, raw: &str, exit_code: Option<i32>) -> Option<FilterOutcome> {
    let trimmed = command.trim();
    let tail = trimmed.rsplit("&&").next().unwrap_or(trimmed).trim();

    cargo::try_filter(tail, raw, exit_code).or_else(|| git::try_filter(tail, raw, exit_code))
}

/// True when `new` is smaller than `raw` by at least 10% — the bar for a smart
/// filter to bother replacing output with a summary/trim instead of leaving it
/// untouched (falls through to spec table / passthrough).
fn shrunk_enough(raw: &str, new: &str) -> bool {
    if raw.is_empty() {
        return false;
    }
    (new.len() as f64) <= (raw.len() as f64) * 0.9
}

/// Wrap a sub-filter's result: `None` if it didn't change anything or the
/// change isn't meaningfully smaller; otherwise a "changed" [`FilterOutcome`].
fn finalize(raw: &str, text: String, name: &'static str) -> Option<FilterOutcome> {
    if text == raw || !shrunk_enough(raw, &text) {
        return None;
    }
    Some(FilterOutcome {
        text,
        filter_name: Some(name),
        changed: true,
    })
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;

//! Stall detection: interstitial narration vs a finished report.

/// Returns `true` when `text` looks like interstitial narration rather than a
/// finished report — e.g. "Let me read a few more files:" — so the engine can
/// nudge the model to keep going instead of accepting the half-thought as done.
///
/// Altitude-aware: a substantial or structured response (long, multi-line, or
/// containing markdown headings/tables/lists) is NEVER a stall. Only short,
/// bodyless lead-ins or dangling colons qualify.
///
/// Criteria for NOT a stall (any one is enough to return false):
/// - trimmed length >= 300 chars
/// - contains a newline (multi-line = has a body)
/// - contains "##" (markdown heading)
/// - contains "| " (table row)
/// - contains "- " (list item)
///
/// A stall requires ALL of the following (after ruling out the above):
/// - trimmed text is empty, OR
/// - trimmed text ends with `:` (classic "Let me read…:" cliffhanger), OR
/// - trimmed text starts with a known procrastination phrase (case-insensitive)
pub(crate) fn is_stall(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    // Substantial -> long, multi-line, or structured (headings/tables/lists). Never a stall.
    let substantial = t.len() >= 300
        || t.contains('\n')
        || t.contains("##")
        || t.contains("| ")
        || t.contains("- ");
    if substantial {
        return false;
    }
    // Short + bodyless: a "let me..."/"next I..." lead-in or a dangling colon.
    let lower = t.to_lowercase();
    let lead_in = [
        "let me", "i'll", "i will", "let's", "now i", "next,", "next i", "first,",
    ]
    .iter()
    .any(|p| lower.starts_with(p));
    t.ends_with(':') || lead_in
}

#[cfg(test)]
#[path = "stall_test.rs"]
mod tests;

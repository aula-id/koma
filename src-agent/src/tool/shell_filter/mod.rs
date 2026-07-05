//! Shell output filtering for the `bash` tool's "saving" mode: trims known-noisy
//! command output (npm install, pip install, docker build, etc.) so verbose
//! build logs don't flood the context. Only invoked when
//! [`crate::tool::shell::OutputOpts::saving`] is true; passthrough otherwise.

pub mod spec;
mod smart;

/// Result of running a command's output through the filter pipeline.
pub struct FilterOutcome {
    pub text: String,
    pub filter_name: Option<&'static str>,
    pub changed: bool,
}

/// Filter `raw` output for `command` (the full shell command as run, may
/// contain `&&`/pipes) given its `exit_code`. Dispatch order:
/// 1. Smart filters (semantic, command-family-aware — cargo/git).
/// 2. Static spec table (`spec::table()`) — first regex match on the trimmed
///    command wins.
/// 3. Passthrough — output returned unchanged.
///
/// Commands containing a pipe are never filtered: the user already reshaped
/// the output themselves (e.g. `| grep`, `| jq`), so second-guessing it here
/// would fight their intent.
pub fn filter_output(command: &str, raw: &str, exit_code: Option<i32>) -> FilterOutcome {
    if command.contains('|') {
        return FilterOutcome { text: raw.to_string(), filter_name: None, changed: false };
    }

    // 1. Smart filters — semantic, command-family-aware.
    if let Some(outcome) = smart::try_smart(command, raw, exit_code) {
        return outcome;
    }

    // 2. Static spec table — first command-regex match wins.
    let trimmed = command.trim();
    for s in spec::table() {
        if s.match_command.is_match(trimmed) {
            return match spec::apply(s, raw, exit_code) {
                Some(text) => FilterOutcome { text, filter_name: Some(s.name), changed: true },
                None => FilterOutcome { text: raw.to_string(), filter_name: None, changed: false },
            };
        }
    }

    // 3. Passthrough.
    FilterOutcome { text: raw.to_string(), filter_name: None, changed: false }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;

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
mod tests {
    use super::*;

    #[test]
    fn pipe_command_passes_through_untouched() {
        let raw = "npm warn deprecated foo\nadded 3 packages in 2s\n";
        let outcome = filter_output("npm install | tee log.txt", raw, Some(0));
        assert!(!outcome.changed);
        assert_eq!(outcome.filter_name, None);
        assert_eq!(outcome.text, raw);
    }

    #[test]
    fn unmatched_command_passes_through() {
        let raw = "hello\nworld\n";
        let outcome = filter_output("echo hello world", raw, Some(0));
        assert!(!outcome.changed);
        assert_eq!(outcome.filter_name, None);
        assert_eq!(outcome.text, raw);
    }

    #[test]
    fn npm_install_realistic_fixture_strips_noise_keeps_summary() {
        let raw = "\
npm timing npm:load:whichnode Completed in 1ms
npm http fetch GET 200 https://registry.npmjs.org/foo 120ms
npm warn deprecated foo@1.0.0: use bar instead
added 42 packages in 3.2s
2 packages are looking for funding
";
        let outcome = filter_output("cd project && npm install", raw, Some(0));
        assert!(outcome.changed);
        assert_eq!(outcome.filter_name, Some("npm-install"));
        assert!(!outcome.text.contains("npm timing"));
        assert!(!outcome.text.contains("npm http"));
        assert!(!outcome.text.contains("npm warn deprecated"));
        assert!(outcome.text.contains("added 42 packages in 3.2s"));
        assert!(outcome.text.contains("2 packages are looking for funding"));
    }

    #[test]
    fn pip_install_realistic_fixture_strips_noise_keeps_summary() {
        let raw = "\
Collecting requests
Downloading requests-2.31.0-py3-none-any.whl (62 kB)
Using cached idna-3.4-py3-none-any.whl
Installing collected packages: idna, requests
Successfully installed idna-3.4 requests-2.31.0
";
        let outcome = filter_output("pip install requests", raw, Some(0));
        assert!(outcome.changed);
        assert_eq!(outcome.filter_name, Some("pip-install"));
        assert!(!outcome.text.contains("Collecting"));
        assert!(!outcome.text.contains("Downloading"));
        assert!(!outcome.text.contains("Using cached"));
        assert!(!outcome.text.contains("Installing collected"));
        assert!(outcome.text.contains("Successfully installed idna-3.4 requests-2.31.0"));
    }

    #[test]
    fn non_zero_exit_relaxes_caps_4x() {
        // max_lines for npm-install is 40; with 200 lines of plain (non-strippable)
        // content and a non-zero exit, the cap relaxes to 160, so more survives
        // than the zero-exit case would allow.
        let mut raw = String::new();
        for i in 0..200 {
            raw.push_str(&format!("line {i}\n"));
        }
        let ok = filter_output("npm install", &raw, Some(0));
        let err = filter_output("npm install", &raw, Some(1));
        let ok_lines = ok.text.lines().count();
        let err_lines = err.text.lines().count();
        assert!(err_lines > ok_lines);
    }
}
